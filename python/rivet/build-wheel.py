#!/usr/bin/env python3
"""Build the `rivet-hdl` wheel, binaries and all.

maturin builds one crate. The wheel needs three artifacts: the `_rivet`
extension module (maturin's job), the `rivet` CLI, and the PLI plugin the
simulator loads. This builds the other two, stages them where the package
expects them, and hands over to maturin.

    python3 build-wheel.py                     # a wheel for this interpreter
    python3 build-wheel.py --out dist --sdist  # and a source distribution

The plugin links libpython, so a wheel is specific to one Python minor
version as well as one platform — run this once per interpreter you want
to support. See `crates/plugin/src/lib.rs` for why abi3 does not help.
"""

from __future__ import annotations

import argparse
import os
import platform
import shutil
import subprocess
import sys
from pathlib import Path

HERE = Path(__file__).resolve().parent
ROOT = HERE.parent.parent
BIN_DIR = HERE / "src" / "rivet" / "_bin"


def dylib(name: str) -> str:
    return f"lib{name}.dylib" if sys.platform == "darwin" else f"lib{name}.so"


def strip(path: Path) -> None:
    """Drop debug info from a staged binary.

    The release profile keeps `debug = 1` so a crash inside a simulator
    can be read; that is worth 30 MB in a checkout and not in a wheel.
    Only debug sections go — the dynamic symbols the simulator resolves
    (`vlog_startup_routines`) and the interpreter's `PyInit__rivet` stay.
    """
    flag = "-S" if sys.platform == "darwin" else "--strip-debug"
    try:
        subprocess.run(["strip", flag, str(path)], check=True,
                       stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    except (OSError, subprocess.CalledProcessError):
        print(f"  note: could not strip {path.name}; the wheel will be larger", file=sys.stderr)


def run(cmd, cwd, env=None):
    print("+", " ".join(str(c) for c in cmd), flush=True)
    subprocess.run(cmd, cwd=cwd, check=True, env={**os.environ, **(env or {})})


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--out", default="dist", help="where to put the wheel (default: dist)")
    ap.add_argument("--debug", action="store_true", help="build unoptimised, for a quick check")
    ap.add_argument("--sdist", action="store_true", help="also build a source distribution")
    ap.add_argument("--interpreter", default=sys.executable,
                    help="build for this interpreter (default: the one running this script)")
    ap.add_argument("--no-strip", action="store_true",
                    help="keep debug info in the bundled binaries (about 30 MB larger)")
    args = ap.parse_args()

    profile = [] if args.debug else ["--release"]
    where = "debug" if args.debug else "release"
    env = {"PYO3_PYTHON": args.interpreter}

    # 1. The CLI. A plain Rust binary from the main workspace, no Python.
    run(["cargo", "build", *profile, "-p", "rivet-hdl-cli"], cwd=ROOT)
    cli = ROOT / "target" / where / "rivet"

    # 2. The PLI plugin, with an interpreter embedded. Its own target
    #    directory: it must not share features with the extension module,
    #    which is built with `extension-module` and so does not link
    #    libpython at all.
    plugin_target = HERE / "target" / "vpi"
    run(["cargo", "build", *profile, "-p", "rivet-python-plugin"], cwd=HERE,
        env={**env, "CARGO_TARGET_DIR": str(plugin_target)})
    plugin = plugin_target / where / dylib("rivet_python")

    for f in (cli, plugin):
        if not f.exists():
            print(f"build produced no {f}", file=sys.stderr)
            return 1

    # 3. Stage them where `rivet._cli` looks.
    if BIN_DIR.exists():
        shutil.rmtree(BIN_DIR)
    BIN_DIR.mkdir(parents=True)
    for f in (cli, plugin):
        dest = BIN_DIR / f.name
        shutil.copy2(f, dest)
        dest.chmod(0o755)
        before = dest.stat().st_size
        if not args.no_strip:
            strip(dest)
        after = dest.stat().st_size
        note = "" if after == before else f", from {before // 1024} KiB"
        print(f"  staged {dest.relative_to(HERE)}  ({after // 1024} KiB{note})")

    # 4. The wheel.
    run(["maturin", "build", *profile, "-o", args.out, "--interpreter", args.interpreter], cwd=HERE, env=env)
    if args.sdist:
        run(["maturin", "sdist", "-o", args.out], cwd=HERE, env=env)

    print(f"\n{platform.python_version()} on {platform.machine()}: wheel in {args.out}/")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
