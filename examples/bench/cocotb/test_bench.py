"""cocotb equivalents of the Rivet benchmarks in ../src/lib.rs."""
import os
import time

import cocotb
from cocotb.clock import Clock
from cocotb.triggers import ReadOnly, RisingEdge

N = int(os.environ.get("RIVET_BENCH_N", "100000"))


def report(name, cycles, t0):
    secs = time.perf_counter() - t0
    cocotb.log.info(
        f"BENCH {name}: {cycles} cycles in {secs:.3f}s = {cycles / secs:.0f} cycles/s, {secs * 1e6 / cycles:.2f} us/cycle"
    )


@cocotb.test()
async def edge_roundtrip(dut):
    cocotb.start_soon(Clock(dut.clk, 10, "ns").start())
    t0 = time.perf_counter()
    for _ in range(N):
        await RisingEdge(dut.clk)
    report("edge_roundtrip", N, t0)


@cocotb.test()
async def value_traffic(dut):
    cocotb.start_soon(Clock(dut.clk, 10, "ns").start())
    acc = 0
    t0 = time.perf_counter()
    for i in range(N):
        await RisingEdge(dut.clk)
        dut.din.value = i
        acc += dut.dout.value.to_unsigned() if dut.dout.value.is_resolvable else 0
        w = dut.wide.value
        acc += (w.to_unsigned() & 0xFFFFFFFF) if w.is_resolvable else 0
    report("value_traffic", N, t0)
    assert acc != 0


@cocotb.test()
async def edge_then_readonly(dut):
    cocotb.start_soon(Clock(dut.clk, 10, "ns").start())
    last = 0
    t0 = time.perf_counter()
    for i in range(N):
        await RisingEdge(dut.clk)
        dut.din.value = i
        await ReadOnly()
        v = dut.dout.value.to_unsigned()
        if i > 1:
            assert v == last + 1
        last = v
    report("edge_then_readonly", N, t0)


@cocotb.test()
async def many_tasks(dut):
    cocotb.start_soon(Clock(dut.clk, 10, "ns").start())
    n = N // 10

    async def mon():
        for _ in range(n):
            await RisingEdge(dut.clk)

    t0 = time.perf_counter()
    tasks = [cocotb.start_soon(mon()) for _ in range(100)]
    for t in tasks:
        await t
    report("many_tasks(100 tasks)", n, t0)
