#!/usr/bin/env python3
"""Measure edit-to-result latency: what a testbench author waits for.

Three edits, timed per simulator:

  cold      everything rebuilt (cargo clean plus a fresh design build)
  test      one line changed in the test crate, design untouched
  hdl       one line changed in the HDL, test crate untouched

    ci/loop-latency.py --sim icarus --example dff
"""
import argparse
import re
import shutil
import subprocess
import sys
import time
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent


def run(cmd, **kw):
    return subprocess.run(cmd, capture_output=True, text=True, **kw)


def timed(cmd) -> tuple[float, bool]:
    start = time.monotonic()
    r = run(cmd)
    return time.monotonic() - start, r.returncode == 0


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--sim", default="icarus")
    ap.add_argument("--example", default="dff")
    ap.add_argument("--filter", default=None, help="run one test, to time the loop not the suite")
    args = ap.parse_args()

    crate = ROOT / "examples" / args.example
    lib = crate / "src" / "lib.rs"
    hdl = sorted((crate / "hdl").glob("*"))[0]
    rivet = ROOT / "target" / "debug" / "rivet"
    cmd = [str(rivet), "run", "--sim", args.sim, "-C", str(crate)]
    if args.filter:
        cmd += ["--filter", args.filter]

    lib_backup = lib.read_text()
    hdl_backup = hdl.read_text()
    results = {}
    try:
        # Cold: no cargo artefacts for the crate, no design build.
        run(["cargo", "clean", "-p", crate.name.replace("_", "-").join(["example-", ""]) if False else f"example-{args.example.replace('_', '-')}"], cwd=ROOT)
        shutil.rmtree(crate / "sim_build", ignore_errors=True)
        results["cold"], ok = timed(cmd)
        if not ok:
            print("cold run failed", file=sys.stderr)
            return 1
        # Warm with no edit at all, for the floor.
        results["nothing"], _ = timed(cmd)
        # A test-crate edit.
        lib.write_text(lib_backup + f"\n// edit {time.time()}\n")
        results["test"], _ = timed(cmd)
        # An HDL edit: a comment, so behaviour does not change.
        hdl.write_text(hdl_backup + f"\n// edit {time.time()}\n")
        results["hdl"], _ = timed(cmd)
    finally:
        lib.write_text(lib_backup)
        hdl.write_text(hdl_backup)

    print(f"\n{args.example} on {args.sim}\n")
    print(f"{'edit':<10} {'seconds':>9}")
    for k in ("cold", "nothing", "test", "hdl"):
        print(f"{k:<10} {results[k]:>9.1f}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
