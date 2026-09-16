"""UART: talking to a wire with a stopwatch, not a clock.

What this example is for: the shape of testbench where the protocol lives
in *time* rather than in clock edges. Nothing here awaits the design's
clock -- the testbench measures bit periods with `rivet.timer`, which is
how you would describe a serial line to a person.

    rivet run --python --sim icarus
"""

import rivet

BIT = "1us"          # one bit at 1 Mbaud
HALF_BIT = "500ns"
IDLE = 1


async def reset(dut, clk):
    dut.signal("rst_n").set(0)
    dut.signal("tx_start").set(0)
    dut.signal("rx_in").set(IDLE)
    dut.signal("loopback").set(0)
    await clk.rising_edge(n=3)
    dut.signal("rst_n").set(1)
    await clk.rising_edge()
    await rivet.next_time_step()


async def send_frame(rx_in, byte):
    """Bit-bang one 8N1 frame onto a wire, in Python.

    Start bit, eight data bits least-significant first, stop bit -- each
    held for exactly one bit period. This is the whole protocol, and it
    reads like the timing diagram.
    """
    rx_in.set(0)                       # start
    await rivet.timer(BIT)
    for i in range(8):
        rx_in.set((byte >> i) & 1)
        await rivet.timer(BIT)
    rx_in.set(1)                       # stop
    await rivet.timer(BIT)


async def recv_frame(tx_out):
    """Decode one 8N1 frame off a wire by sampling in the middle of each bit."""
    # Wait for the line to fall out of idle.
    await tx_out.falling_edge()
    # Half a bit in puts us at the middle of the start bit; one more bit
    # period per sample keeps us centred from then on.
    await rivet.timer(HALF_BIT)
    assert tx_out.get() == 0, "start bit was not low at its midpoint"

    byte = 0
    for i in range(8):
        await rivet.timer(BIT)
        byte |= tx_out.get() << i
    await rivet.timer(BIT)
    assert tx_out.get() == 1, "stop bit was not high"
    return byte


@rivet.test(timeout="1ms")
async def transmitted_bytes_appear_on_the_wire(dut):
    """Decode the transmitter's output in Python and compare."""
    clk = dut.signal("clk")
    rivet.Clock(clk, "20ns").start()
    await reset(dut, clk)

    tx_start, tx_data, tx_busy = (dut.signal(n) for n in ("tx_start", "tx_data", "tx_busy"))
    tx_out = dut.signal("tx_out")

    for byte in (0x00, 0xFF, 0x55, 0xA5, 0x01, 0x80):
        # Start the decoder first: it waits on the wire, not on us.
        decoder = rivet.start_soon(recv_frame(tx_out), name="decoder")

        tx_data.set(byte)
        tx_start.set(1)
        await clk.rising_edge()
        await rivet.next_time_step()
        tx_start.set(0)

        got = await decoder
        assert got == byte, f"sent {byte:#04x}, the wire carried {got:#04x}"

        # Let the line settle back to idle before the next frame.
        await rivet.timer(BIT)
        assert tx_busy.get() == 0, "still busy after a full frame"


@rivet.test(timeout="1ms")
async def received_bytes_come_off_the_wire(dut):
    """Bit-bang frames into the receiver and check what it reports."""
    clk = dut.signal("clk")
    rivet.Clock(clk, "20ns").start()
    await reset(dut, clk)

    rx_in, rx_data, rx_valid = (dut.signal(n) for n in ("rx_in", "rx_data", "rx_valid"))

    async def collect(n):
        """Watch `rx_valid` and gather the bytes the receiver reports."""
        out = []
        while len(out) < n:
            await clk.rising_edge()
            await rivet.read_only()
            if rx_valid.get():
                out.append(rx_data.get())
            await rivet.next_time_step()
        return out

    sent = [0x3C, 0x00, 0xFF, 0x7E]
    watcher = rivet.start_soon(collect(len(sent)), name="rx-watch")

    for byte in sent:
        await send_frame(rx_in, byte)
        await rivet.timer(BIT)          # idle between frames

    got = await rivet.with_timeout(watcher.join(), "200us")
    assert got == sent, f"sent {[hex(b) for b in sent]}, received {[hex(b) for b in got]}"


@rivet.test(timeout="2ms")
async def loopback_round_trips_every_byte(dut):
    """With the receiver tied to our own transmitter, TX and RX check each
    other -- neither side is modelled by the testbench."""
    clk = dut.signal("clk")
    rivet.Clock(clk, "20ns").start()
    await reset(dut, clk)
    dut.signal("loopback").set(1)

    tx_start, tx_data, tx_busy = (dut.signal(n) for n in ("tx_start", "tx_data", "tx_busy"))
    rx_data, rx_valid = dut.signal("rx_data"), dut.signal("rx_valid")

    sb = rivet.Scoreboard("uart-loopback")
    rng = rivet.Rng(seed=rivet.test_seed())
    bytes_to_send = [rng.randint(0, 255) for _ in range(24)]

    async def collect(n):
        out = []
        while len(out) < n:
            await clk.rising_edge()
            await rivet.read_only()
            if rx_valid.get():
                out.append(rx_data.get())
            await rivet.next_time_step()
        return out

    watcher = rivet.start_soon(collect(len(bytes_to_send)), name="rx-watch")

    for byte in bytes_to_send:
        sb.expect(byte)
        tx_data.set(byte)
        tx_start.set(1)
        await clk.rising_edge()
        await rivet.next_time_step()
        tx_start.set(0)
        # Wait out the frame: ten bit periods plus a margin.
        await rivet.timer("11us")
        assert tx_busy.get() == 0, f"transmitter still busy after sending {byte:#04x}"

    for b in await rivet.with_timeout(watcher.join(), "500us"):
        sb.observe(b)
    sb.finish()


@rivet.test(timeout="1ms")
async def a_start_bit_that_is_noise_is_ignored(dut):
    """A glitch shorter than half a bit must not be taken for a frame."""
    clk = dut.signal("clk")
    rivet.Clock(clk, "20ns").start()
    await reset(dut, clk)

    rx_in, rx_valid = dut.signal("rx_in"), dut.signal("rx_valid")

    # A 100 ns dip -- a tenth of a bit.
    rx_in.set(0)
    await rivet.timer("100ns")
    rx_in.set(1)

    # Give it a couple of frame times to do the wrong thing.
    saw_valid = False
    for _ in range(12):
        await rivet.timer(BIT)
        if rx_valid.get_lossy():
            saw_valid = True
    assert not saw_valid, "a 100 ns glitch was decoded as a byte"
