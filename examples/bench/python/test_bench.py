"""Rivet-Python equivalents of the benchmarks in ../src/lib.rs and
../cocotb/test_bench.py.

Three implementations of the same work, on the same design and the same
simulator: Rust on Rivet, Python on Rivet, Python on cocotb. Where they
differ in what they measure it is noted; otherwise they are line for
line the same testbench.
"""

import os
import time

import rivet

N = int(os.environ.get("RIVET_BENCH_N", "100000"))


def report(name, cycles, t0):
    secs = time.perf_counter() - t0
    rivet.log.info(
        f"BENCH {name}: {cycles} cycles in {secs:.3f}s = {cycles / secs:.0f} cycles/s, "
        f"{secs * 1e6 / cycles:.2f} us/cycle"
    )


@rivet.test
async def edge_roundtrip(dut):
    clk = dut.signal("clk")
    rivet.Clock(clk, "10ns").start()
    t0 = time.perf_counter()
    for _ in range(N):
        await clk.rising_edge()
    report("edge_roundtrip", N, t0)


@rivet.test
async def value_traffic(dut):
    clk = dut.signal("clk")
    din, dout, wide = dut.signal("din"), dut.signal("dout"), dut.signal("wide")
    rivet.Clock(clk, "10ns").start()
    acc = 0
    t0 = time.perf_counter()
    for i in range(N):
        await clk.rising_edge()
        din.set(i)
        acc += dout.get_lossy()
        acc += wide.get_lossy() & 0xFFFFFFFF
    report("value_traffic", N, t0)
    assert acc != 0


@rivet.test
async def edge_then_readonly(dut):
    clk = dut.signal("clk")
    din, dout = dut.signal("din"), dut.signal("dout")
    rivet.Clock(clk, "10ns").start()
    last = 0
    t0 = time.perf_counter()
    for i in range(N):
        await clk.rising_edge()
        din.set(i)
        await rivet.read_only()
        v = dout.get()
        if i > 1:
            assert v == last + 1, f"dout {v} != {last + 1} at cycle {i}"
        last = v
    report("edge_then_readonly", N, t0)


@rivet.test
async def many_tasks(dut):
    clk = dut.signal("clk")
    rivet.Clock(clk, "10ns").start()
    n = N // 10

    async def mon():
        for _ in range(n):
            await clk.rising_edge()

    t0 = time.perf_counter()
    tasks = [rivet.start_soon(mon()) for _ in range(100)]
    for t in tasks:
        await t
    report("many_tasks(100 tasks)", n, t0)


@rivet.test
async def timer_only(dut):
    t0 = time.perf_counter()
    for _ in range(N):
        await rivet.timer("10ns")
    report("timer_only", N, t0)


@rivet.test
async def clock_only(dut):
    """The clock runs; nobody awaits it. In Rivet the clock is a native
    task, so this is the same work in Python as it is in Rust."""
    clk = dut.signal("clk")
    rivet.Clock(clk, "10ns").start()
    t0 = time.perf_counter()
    await rivet.timer(f"{N * 10}ns")
    report("clock_only", N, t0)


@rivet.test
async def batched_edges(dut):
    """The same N cycles as `edge_roundtrip`, waited for in one `await`.

    Rivet only: this is the shape that makes a Python testbench cost what
    a Rust one costs, because the interpreter is entered once instead of
    N times. There is no cocotb equivalent.
    """
    clk = dut.signal("clk")
    rivet.Clock(clk, "10ns").start()
    t0 = time.perf_counter()
    await clk.rising_edge(n=N)
    report("batched_edges", N, t0)
