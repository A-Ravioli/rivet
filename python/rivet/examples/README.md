# Example designs and their testbenches

Four small designs, each chosen because its testbench has a different
shape. Every one runs as it stands:

```sh
rivet run --python --sim icarus -C python/rivet/examples/alu
```

| example | the design | what the testbench is for |
|---|---|---|
| [`counter`](counter) | an 8-bit counter with load and enable | the smallest complete testbench: a clock, a reset, a per-cycle check, and a batched wait |
| [`alu`](alu) | a registered 32-bit ALU, eight operations | a **reference model in Python**, seeded random stimulus, and functional coverage that has to reach 100% |
| [`fifo`](fifo) | a synchronous FIFO, show-ahead reads | **two concurrent tasks** — a producer and a consumer that stall independently — checked by a scoreboard |
| [`uart`](uart) | a UART transmitter and receiver, 8N1 | a protocol that lives in **time rather than clock edges**: bit periods measured with `rivet.timer` |

## What each one is worth reading for

**`counter`** — start here. Clock, reset, a loop that checks the count
every cycle, and one `await clk.rising_edge(n=256)` that skips 256 cycles
for the cost of a single trigger.

**`alu`** — the case for writing testbenches in Python at all. The
reference model is eight lines of Python that read like the specification,
and the stimulus is drawn from *interesting* operand classes (zero, one,
all-ones, the sign bit) rather than uniformly, because uniform 32-bit
randomness almost never produces the values an ALU gets wrong. The
covergroup then proves the classes were actually hit, and the test fails
if any opcode went unexercised.

**`fifo`** — why `start_soon` exists. The producer and consumer decide
independently every cycle whether to stall, which is a worse thing to
write as one interleaved loop. The scoreboard checks that everything that
went in came out in order; the covergroup checks the occupancy reached
both empty and full, which random traffic at similar rates does not do on
its own — the consumer deliberately starves the FIFO first.

**`uart`** — nothing in the decoder awaits the design's clock. It waits
for the line to fall, moves half a bit in, and samples every bit period
after that, which is how you would describe a serial line to a person.
The loopback test then ties the receiver to the transmitter so the two
halves check each other with no model in between.

## The timing idiom they all use

```python
await clk.rising_edge()      # the edge: the design has not reacted yet
await rivet.read_only()      # values have settled; check them here
await rivet.next_time_step() # writes are legal again; drive the next cycle
```

An edge trigger returns *before* the design reacts to it, so a check
placed straight after `rising_edge()` sees the previous cycle's values.
That is deliberate and matches cocotb. `docs/python.md` has the whole
timing model.

## Running them

```sh
# One example.
rivet run --python --sim icarus -C python/rivet/examples/fifo

# One test out of it, replayable.
rivet run --python --sim icarus -C python/rivet/examples/alu \
    --filter randomised --seed 12345

# With a coverage floor, which the ALU and FIFO both meet.
rivet run --python --sim icarus -C python/rivet/examples/alu --cov-threshold 100
```

`--python` is implied by the `[python]` section in each `rivet.toml`, so
you can leave it off. GHDL and NVC work the same way for VHDL designs;
Verilator does not, because it has no PLI to load the plugin through.
