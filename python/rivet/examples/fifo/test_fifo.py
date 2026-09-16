"""FIFO: two concurrent tasks, backpressure, and a scoreboard.

What this example is for: the shape of testbench where a producer and a
consumer run independently against the same design and the question is
whether everything that went in comes out, in order, under arbitrary
stalls on both sides.

    rivet run --python --sim icarus
"""

import rivet

DEPTH = 16
WIDTH = 8


async def reset(dut, clk):
    dut.signal("rst_n").set(0)
    dut.signal("wr_en").set(0)
    dut.signal("rd_en").set(0)
    await clk.rising_edge(n=2)
    dut.signal("rst_n").set(1)
    await clk.rising_edge()
    await rivet.next_time_step()


async def step(clk):
    """Advance one cycle and settle, leaving time to drive the next one.

    The edge returns before the design reacts to it, so a testbench that
    wants to see the *result* of the edge waits for ReadOnly, and a
    testbench that wants to drive the next cycle then waits for the next
    time step. Wrapping it once keeps the three lines out of every loop.
    """
    await clk.rising_edge()
    await rivet.read_only()


@rivet.test(timeout="1ms")
async def words_come_out_in_order(dut):
    """The simplest thing that could be wrong: push some, pop them back."""
    clk = dut.signal("clk")
    rivet.Clock(clk, "10ns").start()
    await reset(dut, clk)

    wr_en, wr_data = dut.signal("wr_en"), dut.signal("wr_data")
    rd_en, rd_data = dut.signal("rd_en"), dut.signal("rd_data")
    empty = dut.signal("empty")

    words = [0x11, 0x22, 0x33, 0x44]
    for w in words:
        wr_en.set(1)
        wr_data.set(w)
        await step(clk)
        await rivet.next_time_step()
    wr_en.set(0)

    got = []
    for _ in words:
        await step(clk)
        assert empty.get() == 0, "FIFO went empty with words still in it"
        got.append(rd_data.get())
        await rivet.next_time_step()
        rd_en.set(1)
        await step(clk)
        await rivet.next_time_step()
        rd_en.set(0)

    assert got == words, f"out of order: put in {words}, got back {got}"


@rivet.test(timeout="1ms")
async def full_and_empty_assert_at_the_boundaries(dut):
    clk = dut.signal("clk")
    rivet.Clock(clk, "10ns").start()
    await reset(dut, clk)

    wr_en, wr_data = dut.signal("wr_en"), dut.signal("wr_data")
    rd_en = dut.signal("rd_en")
    full, empty, count = dut.signal("full"), dut.signal("empty"), dut.signal("count")

    await step(clk)
    assert empty.get() == 1, "a FIFO out of reset should be empty"
    assert full.get() == 0
    assert count.get() == 0
    await rivet.next_time_step()

    # Fill it exactly.
    for i in range(DEPTH):
        wr_en.set(1)
        wr_data.set(i)
        await step(clk)
        await rivet.next_time_step()
    wr_en.set(0)
    await step(clk)
    assert full.get() == 1, f"should be full after {DEPTH} writes, count={count.get()}"
    assert count.get() == DEPTH
    await rivet.next_time_step()

    # A write while full must be dropped, not wrap the pointer.
    wr_en.set(1)
    wr_data.set(0xFF)
    await step(clk)
    await rivet.next_time_step()
    wr_en.set(0)
    await step(clk)
    assert count.get() == DEPTH, "a write while full changed the occupancy"
    await rivet.next_time_step()

    # Drain it exactly, checking the data survived the overflow attempt.
    for i in range(DEPTH):
        assert dut.signal("rd_data").get() == i, f"word {i} was corrupted"
        rd_en.set(1)
        await step(clk)
        await rivet.next_time_step()
    rd_en.set(0)
    await step(clk)
    assert empty.get() == 1, f"should be empty after {DEPTH} reads, count={count.get()}"
    assert count.get() == 0


@rivet.test(timeout="10ms")
async def concurrent_traffic_with_random_stalls(dut):
    """A producer and a consumer, each stalling at random, run as tasks.

    This is where `start_soon` earns its place: the two sides make
    independent decisions every cycle, and writing them as one interleaved
    loop would be a worse description of the same thing.

    The consumer starves the FIFO early on so the occupancy actually
    reaches full -- two random streams at similar rates wander around the
    middle and never visit the interesting ends.
    """
    clk = dut.signal("clk")
    rivet.Clock(clk, "10ns").start()
    await reset(dut, clk)

    wr_en, wr_data, full = dut.signal("wr_en"), dut.signal("wr_data"), dut.signal("full")
    rd_en, rd_data, empty = dut.signal("rd_en"), dut.signal("rd_data"), dut.signal("empty")
    count = dut.signal("count")

    sb = rivet.Scoreboard("fifo")
    cg = rivet.Covergroup("fifo")
    cp_occ = cg.point("occupancy", rivet.Bins()
                      .values("empty", [0])
                      .bin("low", 1, DEPTH // 4)
                      .bin("mid", DEPTH // 4 + 1, 3 * DEPTH // 4)
                      .bin("high", 3 * DEPTH // 4 + 1, DEPTH - 1)
                      .values("full", [DEPTH]))

    N = 300
    # Two independent streams from the test's seed, so producer and
    # consumer stalls do not correlate and both replay under `--seed`.
    rng_p = rivet.Rng(seed=rivet.test_seed())
    rng_c = rng_p.fork()

    async def producer():
        sent = 0
        while sent < N:
            await step(clk)
            room = full.get() == 0
            await rivet.next_time_step()
            # Only ever assert wr_en when there is room, so no write is
            # silently dropped and the scoreboard stays exact.
            if room and rng_p.gen_bool(0.9):
                value = (sent * 7 + 3) & 0xFF
                wr_en.set(1)
                wr_data.set(value)
                sb.expect(value)
                sent += 1
            else:
                wr_en.set(0)
        # One more cycle so the last write commits, then stop driving.
        await step(clk)
        await rivet.next_time_step()
        wr_en.set(0)

    async def consumer():
        got = 0
        cycle = 0
        while got < N:
            await step(clk)
            cp_occ.sample(count.get())
            head = rd_data.get() if empty.get() == 0 else None
            await rivet.next_time_step()
            cycle += 1
            # Starve it for the first stretch so the FIFO reaches full,
            # then drain faster than the producer fills.
            p_pop = 0.02 if cycle < 40 else 0.75
            if head is not None and rng_c.gen_bool(p_pop):
                # Show-ahead: the word on the output now is the one this
                # `rd_en` pops at the next edge.
                rd_en.set(1)
                sb.observe(head)
                got += 1
            else:
                rd_en.set(0)
        await step(clk)
        await rivet.next_time_step()
        rd_en.set(0)

    p = rivet.start_soon(producer(), name="producer")
    c = rivet.start_soon(consumer(), name="consumer")
    await p
    await c

    sb.finish()
    rivet.log.info("moved %d words, occupancy coverage %.1f%%", N, cp_occ.percent())
    assert cp_occ.percent() == 100.0, f"never saw every occupancy class: {cp_occ.hits()}"
