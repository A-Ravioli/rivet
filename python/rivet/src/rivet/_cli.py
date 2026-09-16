"""The `rivet` command, as installed by the wheel.

The CLI is a Rust binary; the wheel carries it next to this file rather
than reimplementing it, so `pip install rivet-hdl` gives you the same
`rivet` that `cargo install rivet-hdl-cli` does.
"""

from __future__ import annotations

import os
import sys

#: Where the wheel puts the binaries it carries.
BIN_DIR = os.path.join(os.path.dirname(os.path.abspath(__file__)), "_bin")


def cli_path() -> str:
    """The bundled `rivet` executable."""
    return os.path.join(BIN_DIR, "rivet")


def plugin_path() -> str:
    """The bundled PLI plugin the simulator loads."""
    name = "librivet_python.dylib" if sys.platform == "darwin" else "librivet_python.so"
    return os.path.join(BIN_DIR, name)


def _missing(what: str, path: str) -> int:
    print(
        f"rivet: this install has no {what} at {path}.\n"
        "It was probably built as a source distribution, which cannot carry a\n"
        "compiled binary. Install a wheel for your platform and Python version,\n"
        "or work from a checkout (see docs/python.md).",
        file=sys.stderr,
    )
    return 2


def main() -> int:
    """Entry point for the `rivet` console script."""
    exe = cli_path()
    if not os.path.exists(exe):
        return _missing("`rivet` binary", exe)
    # Hand the process over: the CLI streams the simulator's output and its
    # exit code is the regression's, so there is nothing to wrap.
    os.execv(exe, [exe, *sys.argv[1:]])


if __name__ == "__main__":
    raise SystemExit(main())
