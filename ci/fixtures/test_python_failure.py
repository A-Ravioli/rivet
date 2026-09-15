"""A Python testbench that fails on purpose.

CI copies this next to the counter example and checks that the failure
reaches the summary with its Python traceback, rather than passing
quietly or killing the simulator.
"""

import rivet


@rivet.test(timeout="10us")
async def deliberately_wrong(dut):
    clk = dut.signal("clk")
    rivet.Clock(clk, "10ns").start()
    dut.signal("rst_n").set(1)
    dut.signal("en").set(1)
    await clk.rising_edge(n=3)
    await rivet.read_only()
    assert dut.signal("count").get_lossy() == 0xDEAD, "this assertion is meant to fail"
