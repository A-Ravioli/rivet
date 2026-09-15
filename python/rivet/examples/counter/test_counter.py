"""The counter example, as a Rivet testbench in Python.

Run it with::

    rivet run --python --sim icarus
"""

import rivet


async def reset(dut, clk):
    """Hold the counter in reset for two cycles."""
    dut.signal("rst_n").set(0)
    dut.signal("en").set(0)
    dut.signal("load").set(0)
    await clk.rising_edge(n=2)
    dut.signal("rst_n").set(1)
    await clk.rising_edge()


@rivet.test(timeout="100us")
async def counts_when_enabled(dut):
    clk = dut.signal("clk")
    rivet.Clock(clk, "10ns").start()
    await reset(dut, clk)

    count = dut.signal("count")
    dut.signal("en").set(1)
    for i in range(1, 21):
        await clk.rising_edge()
        # The edge returns before the flop updates; read once it settles.
        await rivet.read_only()
        assert count.get() == i, f"cycle {i}: counted {count.get()}"
        await rivet.next_time_step()


@rivet.test(timeout="100us")
async def holds_when_disabled(dut):
    clk = dut.signal("clk")
    rivet.Clock(clk, "10ns").start()
    await reset(dut, clk)

    count = dut.signal("count")
    dut.signal("en").set(1)
    await clk.rising_edge(n=5)
    await rivet.read_only()
    reached = count.get()
    await rivet.next_time_step()

    dut.signal("en").set(0)
    await clk.rising_edge(n=10)
    await rivet.read_only()
    assert count.get() == reached, "the counter moved while disabled"


@rivet.test(timeout="100us")
async def load_overrides_the_count(dut):
    clk = dut.signal("clk")
    rivet.Clock(clk, "10ns").start()
    await reset(dut, clk)

    count = dut.signal("count")
    dut.signal("load").set(1)
    dut.signal("load_value").set(0xA5)
    await clk.rising_edge()
    await rivet.read_only()
    assert count.get() == 0xA5, f"load put {count.get():#x} in the counter"


@rivet.test(timeout="1ms")
async def wraps_at_the_top(dut):
    """A long run: 300 cycles skipped natively, then checked."""
    clk = dut.signal("clk")
    rivet.Clock(clk, "10ns").start()
    await reset(dut, clk)

    count = dut.signal("count")
    dut.signal("en").set(1)
    # One coroutine resume for 256 cycles, not 256 of them.
    await clk.rising_edge(n=256)
    await rivet.read_only()
    assert count.get() == 0, f"an 8-bit counter should wrap to 0, not {count.get()}"
