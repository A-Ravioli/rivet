# Testbenches in Python

Rivet drives simulators from compiled Rust, which is where its speed comes
from. This is the other way in: write the testbench in Python, and keep
the scheduler, the triggers and the value path native.

```python
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
        # The edge returns before the flop updates; read once it settles.
        await rivet.read_only()
        assert count.get() == i, f"cycle {i}"
        await rivet.next_time_step()
```

```toml
# rivet.toml
[design]
top = "counter"
sources = ["counter.v"]

[python]
tests = ["test_counter"]
```

```sh
rivet run --python --sim icarus
```

No Rust crate and no `cargo`. The full reference — the whole API, the
cost model, the cocotb translation table and the measurements — is in
[`docs/python.md`](https://github.com/A-Ravioli/rivet/blob/main/docs/python.md).

## How it differs from cocotb

cocotb puts the interpreter underneath the scheduler: CPython owns the
event loop and the simulator calls into Python on every callback. Rivet
puts it on top. Rivet's executor owns the scheduler, a trigger is a
simulator callback, and a Python coroutine is one more task on it —
entered once per `await` and not at all for the parts that do not need
it.

## What that costs

Same design, same simulator, same machine. Icarus Verilog 12.0, 20 000
cycles, median of 3 runs, µs of wall-clock per simulated cycle:

| benchmark | cocotb 2.1 | Rivet (Python) | Rivet (Rust) |
|---|---|---|---|
| await an edge every cycle | 23.49 | **3.02** | 2.24 |
| edge, write 32-bit, read 32-bit and 512-bit | 267.01 | **8.26** | 6.27 |
| edge, write, `ReadOnly`, read | 37.39 | **5.47** | 3.48 |
| 100 tasks awaiting every edge | 293.95 | **84.88** | 20.61 |

Python costs about 1.3× to 1.6× over Rust on the ordinary shapes, and
about 4× when a hundred tasks each wake every cycle — which is the honest
version of the tradeoff, because that last row is a hundred coroutine
resumes per cycle and nothing can make it not be.

Two rows not in the table say more about how to write one. A clock that
nobody awaits costs the same in Python as in Rust (1.86 µs against 1.79),
because `rivet.Clock` is a native task. And twenty thousand cycles waited
for in a single `await clk.rising_edge(n=20000)` cost 2.20 µs per cycle —
Rust's number — because the interpreter is entered once instead of twenty
thousand times.

So the rule for a fast Python testbench is: do not write a per-cycle
Python loop you do not need, and let the native side run the repetitive
parts. Clocks, `Memory`, `Scoreboard`, `Covergroup` and `Rng` are all
native already.

Reproduce the table with `ci/bench-python.py --repeat 3 --cycles 20000`.

## Which to write

They are the same harness, and they interoperate — a Python test and a
Rust one score against the same scoreboard and merge into the same
coverage database.

Write Rust when the testbench itself is the hot loop, when you want port
names checked by the compiler (`rivet bindgen`), or when you want
`cargo test`. Write Python when the team already writes Python, when the
reference model is Python, or when the edit-run loop matters more than
the last microsecond — there is no compile step at all.
