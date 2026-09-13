#!/usr/bin/env python3
"""Run the benchmarks several times and report median and spread.

One run of a benchmark says very little: the numbers in docs/benchmarks.md
were single runs. This repeats each benchmark, reports the median and the
spread, and can fail a build when a number regresses against a recorded
baseline.

    ci/bench.py --sim icarus --repeat 5 --cycles 100000
    ci/bench.py --sim icarus --baseline docs/bench-baseline.json --tolerance 2.0
"""
import argparse
import json
import re
import statistics
import subprocess
import sys
import time
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
LINE = re.compile(r"BENCH (\S+): (\d+) cycles in [\d.]+s = [\d.]+ cycles/s, ([\d.]+) us/cycle")


def run_rivet(example: str, sim: str, cycles: int, release: bool) -> dict[str, float]:
    cmd = [str(ROOT / "target" / ("release" if release else "debug") / "rivet"),
           "run", "--sim", sim, "-C", str(ROOT / "examples" / example)]
    if release:
        cmd.append("--release")
    out = subprocess.run(cmd, capture_output=True, text=True,
                         env={**__import__("os").environ, "RIVET_BENCH_N": str(cycles)})
    text = out.stdout + out.stderr
    found = {}
    for m in LINE.finditer(text):
        found[m.group(1)] = float(m.group(3))
    if not found:
        print(text[-2000:], file=sys.stderr)
        raise SystemExit(f"no BENCH lines from {example} on {sim}")
    return found


def run_bare_icarus(cycles: int) -> float | None:
    """The same design with no harness, for the overhead fraction."""
    hdl = ROOT / "examples" / "bench_soc" / "hdl"
    out = ROOT / "examples" / "bench_soc" / "sim_build" / "bare.vvp"
    out.parent.mkdir(parents=True, exist_ok=True)
    build = subprocess.run(["iverilog", "-g2012", "-o", str(out), str(hdl / "picorv32.v"),
                            str(hdl / "soc.v"), str(hdl / "tb_soc.v")], capture_output=True, text=True)
    if build.returncode != 0:
        print(build.stderr[-500:], file=sys.stderr)
        return None
    start = time.monotonic()
    r = subprocess.run(["vvp", str(out), f"+cycles={cycles}"], capture_output=True, text=True)
    elapsed = time.monotonic() - start
    if r.returncode != 0:
        return None
    return elapsed * 1e6 / cycles


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--sim", default="icarus")
    ap.add_argument("--repeat", type=int, default=5)
    ap.add_argument("--cycles", type=int, default=100000)
    ap.add_argument("--release", action="store_true")
    ap.add_argument("--examples", default="bench,bench_soc")
    ap.add_argument("--baseline", type=Path)
    ap.add_argument("--tolerance", type=float, default=2.0,
                    help="fail if a median is worse than baseline * tolerance")
    ap.add_argument("--write-baseline", type=Path)
    args = ap.parse_args()

    runs: dict[str, list[float]] = {}
    for example in args.examples.split(","):
        for _ in range(args.repeat):
            for name, us in run_rivet(example, args.sim, args.cycles, args.release).items():
                runs.setdefault(name, []).append(us)

    print(f"\n{args.sim}, {args.cycles} cycles, {args.repeat} runs, "
          f"{'release' if args.release else 'debug'} harness\n")
    print(f"{'benchmark':<34} {'median us/cycle':>16} {'min':>9} {'max':>9} {'spread':>8}")
    summary = {}
    for name, values in sorted(runs.items()):
        med = statistics.median(values)
        lo, hi = min(values), max(values)
        spread = (hi - lo) / med * 100 if med else 0.0
        summary[name] = med
        print(f"{name:<34} {med:>16.2f} {lo:>9.2f} {hi:>9.2f} {spread:>7.1f}%")

    if args.sim == "icarus":
        bare = run_bare_icarus(args.cycles)
        if bare:
            print(f"\nbare simulator (no harness), same design: {bare:.2f} us/cycle")
            for name in ("soc_clock_only", "soc_with_monitor"):
                if name in summary:
                    over = summary[name] - bare
                    print(f"  harness overhead in {name}: {over:.2f} us/cycle "
                          f"({over / summary[name] * 100:.0f}% of the run)")
            summary["bare_soc"] = bare

    if args.write_baseline:
        args.write_baseline.write_text(json.dumps(summary, indent=2, sort_keys=True) + "\n")
        print(f"\nwrote {args.write_baseline}")

    if args.baseline:
        base = json.loads(args.baseline.read_text())
        bad = []
        for name, med in summary.items():
            if name in base and med > base[name] * args.tolerance:
                bad.append(f"{name}: {med:.2f} us/cycle, baseline {base[name]:.2f}")
        if bad:
            print("\nregressions:\n  " + "\n  ".join(bad))
            return 1
        print("\nno regression against the baseline")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
