"""Rivet: hardware testbenches in Python, on a native scheduler.

Write the testbench in Python; the scheduler, the triggers and the value
path are Rust. A test looks like this::

    import rivet

    @rivet.test(timeout="100us")
    async def counts_when_enabled(dut):
        clk = dut.signal("clk")
        rivet.Clock(clk, "10ns").start()

        dut.signal("rst_n").set(0)
        await clk.rising_edge(n=2)
        dut.signal("rst_n").set(1)

        count = dut.signal("count")
        dut.signal("en").set(1)
        for i in range(1, 21):
            await clk.rising_edge()
            # The edge returns before the flop updates; read once it has
            # settled.
            await rivet.read_only()
            assert count.get() == i, f"cycle {i}"
            await rivet.next_time_step()

Run it with ``rivet run --python``.

Two things are worth knowing about the cost model. First, every ``await``
enters the interpreter exactly once, so a tight per-cycle loop is the
expensive shape — ``await clk.rising_edge(n=1000)`` skips a thousand
cycles for the price of one. Second, anything that does not need Python
should not be in Python: `Clock` runs as a native task, and the kit
(`Memory`, `Scoreboard`, `Covergroup`, `Rng`) is native too, so none of
them cost a coroutine resume. `docs/python.md` has the measurements.
"""

from __future__ import annotations

import sys as _sys
from typing import Any, Awaitable, Callable, Optional, Sequence

try:
    # Inside a simulator, the plugin puts the bindings in the inittab
    # before anything can import a wheel's copy.
    import _rivet
except ImportError:  # pragma: no cover - exercised by the wheel, not by CI
    try:
        from . import _rivet  # type: ignore[attr-defined]
    except ImportError as exc:  # pragma: no cover
        raise ImportError(
            "rivet's compiled bindings are missing. Inside a simulator they come "
            "from the PLI plugin; outside one, install the wheel "
            "(`pip install rivet-hdl`) or build it with "
            "`cargo build -p rivet-python-ext` and put the library on sys.path "
            "as `_rivet`."
        ) from exc

__version__ = _rivet.__version__

# --- the design ----------------------------------------------------------
Module = _rivet.Module
Signal = _rivet.Signal
Slice = _rivet.Slice
LogicVec = _rivet.LogicVec

# --- scheduling ----------------------------------------------------------
Trigger = _rivet.Trigger
Task = _rivet.Task
Clock = _rivet.Clock
Duration = _rivet.Duration

timer = _rivet.timer
read_only = _rivet.read_only
read_write = _rivet.read_write
next_time_step = _rivet.next_time_step
yield_now = _rivet.yield_now
clock_cycles = _rivet.clock_cycles
first = _rivet.first
start_soon = _rivet.start_soon

# --- the run -------------------------------------------------------------
now = _rivet.now
now_str = _rivet.now_str
precision = _rivet.precision
simulator = _rivet.simulator
dump_tasks = _rivet.dump_tasks
test_seed = _rivet.test_seed
base_seed = _rivet.base_seed
is_running = _rivet.is_running

# --- ending a test -------------------------------------------------------
SkipTest = _rivet.SkipTest
TestFailure = _rivet.TestFailure
TestSuccess = _rivet.TestSuccess
CancelledError = _rivet.CancelledError

fail = _rivet.fail
skip = _rivet.skip
finish = _rivet.finish
finish_now = _rivet.finish_now
report_failure = _rivet.report_failure

# --- the kit -------------------------------------------------------------
Memory = _rivet.Memory
Rng = _rivet.Rng
Scoreboard = _rivet.Scoreboard
Bins = _rivet.Bins
CoverPoint = _rivet.CoverPoint
Cross = _rivet.Cross
Covergroup = _rivet.Covergroup
coverage_percent = _rivet.coverage_percent
coverage_json = _rivet.coverage_json
coverage_table = _rivet.coverage_table
write_coverage = _rivet.write_coverage
clear_coverage = _rivet.clear_coverage
parse_readmemh = _rivet.parse_readmemh
seed_for_test = _rivet.seed_for_test

# `rivet.mock`: a simulator in process, for trying something out or for
# testing a testbench helper without a simulator installed.
if hasattr(_rivet, "mock"):
    mock = _rivet.mock
    _sys.modules.setdefault(__name__ + ".mock", mock)


def test(
    _func: Optional[Callable[..., Awaitable[Any]]] = None,
    *,
    name: Optional[str] = None,
    timeout: Any = None,
    timeout_units: Optional[str] = None,
    skip: bool = False,
    expect_fail: bool = False,
    expect_fail_msg: Optional[str] = None,
    expect_timeout: bool = False,
    stage: int = 0,
    wall_timeout: Optional[float] = None,
    param_sets: Optional[Sequence[str]] = None,
) -> Callable[..., Any]:
    """Register an ``async def`` as a test.

    Usable bare or with arguments::

        @rivet.test
        async def smoke(dut): ...

        @rivet.test(timeout="10us", stage=1)
        async def long_one(dut): ...

    ``timeout`` is simulated time, not wall-clock: ``"10us"``, or
    ``(10, "us")``. ``wall_timeout`` is the wall-clock one, in seconds.
    ``expect_fail`` inverts the result, and ``expect_fail_msg`` also
    requires the failure to say a particular thing — so a test that
    guards against a regression states what the failure should look
    like.

    The decorated function is returned unchanged, so it stays callable
    and importable like any other.
    """

    def register(func: Callable[..., Awaitable[Any]]) -> Callable[..., Awaitable[Any]]:
        _rivet.register_test(
            func,
            name=name,
            timeout=timeout,
            timeout_units=timeout_units,
            skip=skip,
            expect_fail=expect_fail,
            expect_fail_msg=expect_fail_msg,
            expect_timeout=expect_timeout,
            stage=stage,
            wall_timeout=wall_timeout,
            param_sets=list(param_sets) if param_sets is not None else None,
        )
        return func

    # Bare `@rivet.test`.
    if _func is not None:
        return register(_func)
    return register


async def with_timeout(coro: Awaitable[Any], duration: Any, units: Optional[str] = None) -> Any:
    """Run ``coro`` with a simulated-time limit.

    Returns its value, or raises :class:`TimeoutError` if the limit passes
    first. The coroutine is cancelled when it does.
    """
    task = start_soon(coro, propagate=False)
    which = await first(task.join(), timer(duration, units))
    if which == 1:
        task.cancel()
        raise TimeoutError(f"timed out after {Duration(duration, units)} of simulated time")
    return task.result()


class _Log:
    """Python log messages, carried by Rivet's logger.

    They interleave with the harness's own output and carry the simulated
    timestamp, so a Python line and a Rust line in the same run are
    ordered by when they happened.
    """

    def debug(self, msg: object, *args: object) -> None:
        _rivet.log_message("debug", _fmt(msg, args))

    def info(self, msg: object, *args: object) -> None:
        _rivet.log_message("info", _fmt(msg, args))

    def warning(self, msg: object, *args: object) -> None:
        _rivet.log_message("warning", _fmt(msg, args))

    warn = warning

    def error(self, msg: object, *args: object) -> None:
        _rivet.log_message("error", _fmt(msg, args))


def _fmt(msg: object, args: Sequence[object]) -> str:
    text = str(msg)
    return text % args if args else text


log = _Log()

__all__ = [
    "Bins",
    "CancelledError",
    "Clock",
    "CoverPoint",
    "Covergroup",
    "Cross",
    "Duration",
    "LogicVec",
    "Memory",
    "Module",
    "Rng",
    "Scoreboard",
    "Signal",
    "SkipTest",
    "Slice",
    "Task",
    "TestFailure",
    "TestSuccess",
    "Trigger",
    "base_seed",
    "clear_coverage",
    "clock_cycles",
    "coverage_json",
    "coverage_percent",
    "coverage_table",
    "dump_tasks",
    "fail",
    "finish",
    "finish_now",
    "first",
    "is_running",
    "log",
    "next_time_step",
    "now",
    "now_str",
    "parse_readmemh",
    "precision",
    "read_only",
    "read_write",
    "report_failure",
    "seed_for_test",
    "simulator",
    "skip",
    "start_soon",
    "test",
    "test_seed",
    "timer",
    "with_timeout",
    "write_coverage",
    "yield_now",
]
