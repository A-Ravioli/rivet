"""Path setup and helpers shared by the tests.

The tests run against `rivet.mock`, the pure-Rust simulator Rivet's own
Rust suites use. That covers everything above the PLI boundary — the
coroutine driver, the triggers, the value path, the regression loop —
with no simulator installed. What it does not cover is the VPI and VHPI
glue, which the Rust suites and the example testbenches cover instead.
"""

from __future__ import annotations

import os
import sys

_HERE = os.path.dirname(os.path.abspath(__file__))
_ROOT = os.path.dirname(_HERE)

for _p in (os.path.join(_ROOT, "build"), os.path.join(_ROOT, "src")):
    if _p not in sys.path:
        sys.path.insert(0, _p)

import rivet  # noqa: E402
import _rivet  # noqa: E402


def fresh():
    """Forget tests registered by an earlier case in this process."""
    _rivet.clear_tests()


def counter_design(width=8, top="top"):
    """`clk`, `rst_n`, `en`, `count`: a counter with synchronous
    active-low reset."""
    d = rivet.mock.MockDesign(top)
    sigs = {
        "clk": d.logic("clk", 1),
        "rst_n": d.logic("rst_n", 1),
        "en": d.logic("en", 1),
        "count": d.logic("count", width),
    }
    d.counter(sigs["clk"], sigs["rst_n"], sigs["en"], sigs["count"])
    return d


def dff_design(width=8):
    """`clk`, `d`, `q`, plus a `wide` signal for value round-trips."""
    d = rivet.mock.MockDesign("top")
    clk = d.logic("clk", 1)
    din = d.logic("d", width)
    q = d.logic("q", width)
    d.logic("wide", 512)
    d.logic("byte", 8)
    d.dff(clk, din, q)
    return d


def only(results):
    """The single result of a one-test run."""
    assert len(results) == 1, f"expected one result, got {len(results)}"
    return results[0]


def statuses(results):
    return {(r["module"], r["name"]): r["status"] for r in results}


def assert_passed(case, result):
    case.assertEqual(result["status"], "passed", msg=result["message"])
