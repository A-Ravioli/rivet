"""A cocotb test that uses rivet_py for its scoreboard, random stimulus and
coverage, on the dff example design. Run with `python run.py` (Icarus)."""

import cocotb
from cocotb.clock import Clock
from cocotb.triggers import FallingEdge, ReadOnly, RisingEdge

import rivet_py as rv


@cocotb.test()
async def dff_with_rivet_kit(dut):
    cocotb.start_soon(Clock(dut.clk, 10, unit="ns").start())
    dut.rst_n.value = 0
    dut.d.value = 0
    for _ in range(2):
        await RisingEdge(dut.clk)
    dut.rst_n.value = 1
    await RisingEdge(dut.clk)

    rng = rv.Rng(rv.seed_for_test(1234, "test_dff::dff_with_rivet_kit"))
    sb = rv.Scoreboard("q")
    cg = rv.Covergroup("dff")
    lo_hi = cg.point("d", rv.Bins().bin("low", 0, 127).bin("high", 128, 255))
    for _ in range(50):
        v = rng.randint(0, 255)
        lo_hi.sample(v)
        dut.d.value = v
        sb.expect(v)
        await RisingEdge(dut.clk)
        await ReadOnly()
        sb.observe(int(dut.q.value))
        # Leave the ReadOnly phase before the next write.
        await FallingEdge(dut.clk)
    sb.finish()
    assert cg.percent() == 100.0, rv.coverage_table()
    rv.write_coverage("coverage.json")
