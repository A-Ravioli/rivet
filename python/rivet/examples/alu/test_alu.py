"""ALU: a reference model, seeded random stimulus and functional coverage.

What this example is for: the shape of testbench where the design is easy
to describe in Python and the interesting work is deciding *what* to throw
at it and proving you threw enough.

    rivet run --python --sim icarus
"""

import rivet

# The opcode encoding, mirroring the localparams in alu.v.
ADD, SUB, AND, OR, XOR, SLT, SLL, SRL = range(8)
OP_NAMES = ["ADD", "SUB", "AND", "OR", "XOR", "SLT", "SLL", "SRL"]

WIDTH = 32
MASK = (1 << WIDTH) - 1


def signed(x):
    """Reinterpret a WIDTH-bit pattern as two's complement."""
    return x - (1 << WIDTH) if x & (1 << (WIDTH - 1)) else x


def model(op, a, b):
    """The reference ALU, in Python.

    This is the whole point of writing the testbench in Python: the model
    is eight lines and reads like the specification. It runs in the test's
    own task, so it costs nothing the design is waiting on.
    """
    if op == ADD:
        return (a + b) & MASK
    if op == SUB:
        return (a - b) & MASK
    if op == AND:
        return a & b
    if op == OR:
        return a | b
    if op == XOR:
        return a ^ b
    if op == SLT:
        return 1 if signed(a) < signed(b) else 0
    if op == SLL:
        return (a << (b & 0x1F)) & MASK
    if op == SRL:
        return a >> (b & 0x1F)
    raise AssertionError(f"no model for op {op}")


async def reset(dut, clk):
    dut.signal("rst_n").set(0)
    dut.signal("valid").set(0)
    await clk.rising_edge(n=2)
    dut.signal("rst_n").set(1)
    await clk.rising_edge()


def operand_bins():
    """Interesting operand classes, not just 'a random 32-bit number'.

    Uniform 32-bit randomness almost never produces 0, -1, or a value near
    a sign boundary, which is exactly where an ALU goes wrong.
    """
    return rivet.Bins().values("zero", [0]) \
                       .values("one", [1]) \
                       .values("all_ones", [MASK]) \
                       .values("sign_bit", [1 << (WIDTH - 1)]) \
                       .bin("small", 2, 0xFF) \
                       .bin("large", 0x100, MASK - 1)


# One representative value per class in `operand_bins()`, in the same
# order, so a sweep over these closes every bin and every cross bin.
CLASS_REPS = [0, 1, MASK, 1 << (WIDTH - 1), 0x42, 0x0012_3456]


def pick_operand(rng):
    """Draw from the classes above rather than uniformly."""
    kind = rng.randint(0, 9)
    if kind <= 3:
        return CLASS_REPS[kind]
    if kind <= 6:
        return rng.randint(2, 0xFF)
    return rng.randint(0x100, MASK - 1)


@rivet.test(timeout="10ms")
async def randomised_against_the_model(dut):
    """Every opcode against the Python model, scored and covered.

    Coverage closure is *directed*, not hoped for: a sweep over every
    opcode against every operand class closes the covergroup exactly, and
    the random traffic afterwards is what actually hunts for bugs. Leaving
    it to the random phase alone makes closure depend on the seed, which
    is how a coverage gate becomes a flaky test.
    """
    clk = dut.signal("clk")
    rivet.Clock(clk, "10ns").start()
    await reset(dut, clk)

    valid = dut.signal("valid")
    op_s, a_s, b_s = dut.signal("op"), dut.signal("a"), dut.signal("b")
    result, rvalid = dut.signal("result"), dut.signal("result_valid")

    # Seeded from the test's own stream, so `--seed` replays this exactly.
    rng = rivet.Rng(seed=rivet.test_seed())
    sb = rivet.Scoreboard("alu")

    cg = rivet.Covergroup("alu")
    cp_op = cg.point("op", rivet.Bins().auto("op", 0, 7))
    cp_a = cg.point("a", operand_bins())
    cp_b = cg.point("b", operand_bins())
    cross = cg.cross("op_x_a", [cp_op, cp_a])

    async def apply(op, a, b):
        """Drive one operation and score the result it produces."""
        op_s.set(op)
        a_s.set(a)
        b_s.set(b)
        valid.set(1)
        cp_op.sample(op)
        cp_a.sample(a)
        cp_b.sample(b)
        sb.expect(model(op, a, b))

        await clk.rising_edge()
        # The output register takes f(a, b, op) at this edge, so the answer
        # is there as soon as the values settle.
        await rivet.read_only()
        assert rvalid.get() == 1, "result_valid did not follow valid"
        sb.observe(result.get())
        await rivet.next_time_step()

    # Phase 1: every opcode against every operand class.
    for op in range(8):
        for i, a in enumerate(CLASS_REPS):
            await apply(op, a, CLASS_REPS[(i + 1) % len(CLASS_REPS)])

    assert cross.percent() == 100.0, f"the directed sweep left cross bins open: {cross.hits()}"

    # Phase 2: random traffic, which is what looks for the bugs.
    for _ in range(400):
        await apply(rng.randint(0, 7), pick_operand(rng), pick_operand(rng))

    valid.set(0)
    await clk.rising_edge()
    await rivet.next_time_step()

    sb.finish()
    rivet.log.info("covered %.1f%% of the ALU covergroup", cg.percent())
    assert cg.percent() == 100.0, "the covergroup did not close"


@rivet.test(timeout="1ms")
async def zero_flag_tracks_the_result(dut):
    """A directed test for one output the random run barely touches."""
    clk = dut.signal("clk")
    rivet.Clock(clk, "10ns").start()
    await reset(dut, clk)

    valid, op_s, a_s, b_s = (dut.signal(n) for n in ("valid", "op", "a", "b"))
    result, zero = dut.signal("result"), dut.signal("zero")

    cases = [
        (SUB, 7, 7, True),          # 7 - 7 == 0
        (SUB, 7, 6, False),
        (AND, 0xF0F0, 0x0F0F, True),
        (XOR, 0xABCD, 0xABCD, True),
        (OR, 0, 1, False),
    ]
    for op, a, b, want_zero in cases:
        op_s.set(op)
        a_s.set(a)
        b_s.set(b)
        valid.set(1)
        await clk.rising_edge()
        await rivet.read_only()
        got_zero = zero.get() == 1
        got = result.get()
        assert got == model(op, a, b), f"{OP_NAMES[op]}({a:#x},{b:#x}) = {got:#x}"
        assert got_zero == want_zero, (
            f"{OP_NAMES[op]}({a:#x},{b:#x}) -> zero={got_zero}, expected {want_zero}"
        )
        await rivet.next_time_step()


@rivet.test(timeout="1ms")
async def reset_clears_the_outputs(dut):
    clk = dut.signal("clk")
    rivet.Clock(clk, "10ns").start()
    await reset(dut, clk)

    # Put something non-zero in the output register...
    dut.signal("op").set(OR)
    dut.signal("a").set(0xDEADBEEF)
    dut.signal("b").set(0)
    dut.signal("valid").set(1)
    await clk.rising_edge()
    await rivet.read_only()
    assert dut.signal("result").get() == 0xDEADBEEF
    await rivet.next_time_step()

    # ...then reset and check it goes away.
    dut.signal("rst_n").set(0)
    await clk.rising_edge()
    await rivet.read_only()
    assert dut.signal("result").get() == 0, "reset did not clear the result"
    assert dut.signal("result_valid").get() == 0, "reset did not clear result_valid"


@rivet.test(timeout="1ms")
async def invalid_cycles_hold_the_result(dut):
    """`valid` low must leave the previous result standing."""
    clk = dut.signal("clk")
    rivet.Clock(clk, "10ns").start()
    await reset(dut, clk)

    dut.signal("op").set(ADD)
    dut.signal("a").set(100)
    dut.signal("b").set(23)
    dut.signal("valid").set(1)
    await clk.rising_edge()
    await rivet.read_only()
    held = dut.signal("result").get()
    assert held == 123
    await rivet.next_time_step()

    dut.signal("valid").set(0)
    dut.signal("a").set(0xFFFF)
    # Ten idle cycles in a single await: no Python runs in between.
    await clk.rising_edge(n=10)
    await rivet.read_only()
    assert dut.signal("result").get() == held, "an invalid cycle changed the result"
    assert dut.signal("result_valid").get() == 0
