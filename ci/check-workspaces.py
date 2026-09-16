#!/usr/bin/env python3
"""Every cargo workspace in the tree resolves.

The repository has three: the root one, `python/rivet` (the Python
testbench API) and `python/rivet_py` (the cocotb kit shim). The two under
`python/` are deliberately outside the root workspace, because they need
libpython to build — which also means `cargo test --workspace` never
looks at them, so a manifest that no longer resolves stays broken until a
late CI job happens to build it.

Renaming a crate is the way this happens: a path dependency keyed by the
old package name resolves to nothing, and the error only shows up
wherever that crate is built. This asks cargo to resolve each workspace,
which is quick and needs no compiler.
"""
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
WORKSPACES = [Path("."), Path("python/rivet"), Path("python/rivet_py")]

failed = False
for ws in WORKSPACES:
    d = ROOT / ws
    if not (d / "Cargo.toml").exists():
        print(f"{ws}: no Cargo.toml", file=sys.stderr)
        failed = True
        continue
    r = subprocess.run(["cargo", "metadata", "--format-version", "1", "--offline"],
                       cwd=d, capture_output=True, text=True)
    if r.returncode != 0:
        # Offline resolution fails on a cold registry cache; retry online
        # rather than reporting a network problem as a manifest problem.
        r = subprocess.run(["cargo", "metadata", "--format-version", "1"],
                           cwd=d, capture_output=True, text=True)
    if r.returncode != 0:
        print(f"{ws}: does not resolve\n{r.stderr.strip()}", file=sys.stderr)
        failed = True
    else:
        print(f"{ws}: resolves")

sys.exit(1 if failed else 0)
