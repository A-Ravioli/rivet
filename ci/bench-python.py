#!/usr/bin/env python3
"""Three ways to write the same testbench, measured against each other.

Rust on Rivet, Python on Rivet, and Python on cocotb run the same
benchmarks (`examples/bench/`) on the same design and the same simulator,
in this process's own environment. Each is repeated and the median
reported, because a single run of any of them says very little.

    ci/bench-python.py --repeat 5 --cycles 20000
    ci/bench-python.py --repeat 3 --cycles 5000 --skip cocotb

Needs a simulator (Icarus by default) and, for the cocotb column,
cocotb installed. Anything missing is reported as a gap rather than
silently dropped.
"""
import argparse
import os
import re
import statistics
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
BENCH = re.compile(r"BENCH (.+?): (\d+) cycles in [\d.]+s = [\d.]+ cycles/s, ([\d.]+) us/cycle")

# Benchmarks all three can run, in the order they are reported.
COMMON = ["edge_roundtrip", "value_traffic", "edge_then_readonly", "many_tasks(100 tasks)"]
# Rivet-only shapes, reported separately so nothing looks like a
# comparison that was not made.
RIVET_ONLY = ["timer_only", "clock_only", "batched_edges"]


def parse(text):
    return {m.group(1): float(m.group(3)) for m in BENCH.finditer(text)}


def run(cmd, cycles, cwd=None):
    env = {**os.environ, "RIVET_BENCH_N": str(cycles)}
    out = subprocess.run(cmd, capture_output=True, text=True, cwd=cwd, env=env)
    text = out.stdout + out.stderr
    found = parse(text)
    if not found:
        print(text[-1500:], file=sys.stderr)
        raise SystemExit(f"no BENCH lines from {' '.join(str(c) for c in cmd)}")
    return found


def rivet_binary(release):
    exe = ROOT / "target" / ("release" if release else "debug") / "rivet"
    if not exe.exists():
        raise SystemExit(f"{exe} is missing; run `cargo build {'--release ' if release else ''}-p rivet-hdl-cli`")
    return exe


def run_rust(sim, cycles, release):
    cmd = [str(rivet_binary(release)), "run", "--sim", sim, "-C", str(ROOT / "examples" / "bench")]
    if release:
        cmd.append("--release")
    return run(cmd, cycles)


def run_rivet_python(sim, cycles, release):
    cmd = [str(rivet_binary(release)), "run", "--python", "--sim", sim,
           "-C", str(ROOT / "examples" / "bench")]
    if release:
        cmd.append("--release")
    return run(cmd, cycles)


def run_cocotb(sim, cycles):
    return run([sys.executable, "run.py", sim], cycles, cwd=ROOT / "examples" / "bench" / "cocotb")


def spread(values):
    med = statistics.median(values)
    return med, min(values), max(values)


def table(rows, columns, note=""):
    head = ["benchmark"] + columns
    widths = [max(len(head[i]), *(len(r[i]) for r in rows)) for i in range(len(head))]
    out = ["| " + " | ".join(h.ljust(w) for h, w in zip(head, widths)) + " |",
           "|" + "|".join("-" * (w + 2) for w in widths) + "|"]
    for r in rows:
        out.append("| " + " | ".join(c.ljust(w) for c, w in zip(r, widths)) + " |")
    if note:
        out.append("")
        out.append(note)
    return "\n".join(out)


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--sim", default="icarus")
    ap.add_argument("--cycles", type=int, default=20000)
    ap.add_argument("--repeat", type=int, default=3)
    ap.add_argument("--release", action="store_true", default=True)
    ap.add_argument("--debug", dest="release", action="store_false",
                    help="measure a debug build (much slower; not comparable to published numbers)")
    ap.add_argument("--skip", action="append", default=[],
                    choices=["rust", "python", "cocotb"], help="leave a column out")
    args = ap.parse_args()

    runners = {
        "rust": lambda: run_rust(args.sim, args.cycles, args.release),
        "python": lambda: run_rivet_python(args.sim, args.cycles, args.release),
        "cocotb": lambda: run_cocotb(args.sim, args.cycles),
    }
    labels = {"rust": "Rivet (Rust)", "python": "Rivet (Python)", "cocotb": "cocotb"}

    samples = {}
    gaps = {}
    for key, fn in runners.items():
        if key in args.skip:
            gaps[key] = "skipped"
            continue
        runs = []
        for i in range(args.repeat):
            print(f"  {labels[key]} run {i + 1}/{args.repeat}...", file=sys.stderr)
            try:
                runs.append(fn())
            except SystemExit as e:
                gaps[key] = str(e)
                break
        if runs:
            samples[key] = runs

    order = [k for k in ("cocotb", "python", "rust") if k in samples]
    if not order:
        print("nothing ran", file=sys.stderr)
        return 1

    def cell(key, name):
        values = [r[name] for r in samples[key] if name in r]
        if not values:
            return "—"
        med, lo, hi = spread(values)
        if args.repeat == 1:
            return f"{med:.2f}"
        return f"{med:.2f} ({lo:.2f}–{hi:.2f})"

    print(f"\n{args.sim}, {args.cycles} cycles, median of {args.repeat} runs, µs per cycle\n")
    rows = [[name] + [cell(k, name) for k in order] for name in COMMON]
    # Speed-up against cocotb where both ran.
    columns = [labels[k] for k in order]
    if "cocotb" in samples and "python" in samples:
        columns.append("Python speed-up")
        for row, name in zip(rows, COMMON):
            try:
                c = statistics.median(r[name] for r in samples["cocotb"] if name in r)
                p = statistics.median(r[name] for r in samples["python"] if name in r)
                row.append(f"{c / p:.1f}x")
            except statistics.StatisticsError:
                row.append("—")
    print(table(rows, columns))

    only = [k for k in ("python", "rust") if k in samples]
    if only:
        rows = [[name] + [cell(k, name) for k in only] for name in RIVET_ONLY]
        rows = [r for r in rows if any(c != "—" for c in r[1:])]
        if rows:
            print("\nShapes cocotb has no equivalent for:\n")
            print(table(rows, [labels[k] for k in only]))

    for key, why in gaps.items():
        print(f"\nnot measured: {labels[key]} — {why}", file=sys.stderr)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
