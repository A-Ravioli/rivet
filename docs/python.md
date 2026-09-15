# Testbenches in Python

Rivet drives HDL simulators from compiled Rust. This is the other way in:
write the testbench in Python, and let the scheduler, the triggers and the
value path stay native.

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

```sh
rivet run --python --sim icarus
```

No Rust crate, no `cargo`, no build step you have to think about.

## Why this is faster than cocotb

cocotb puts the interpreter *underneath* the scheduler: CPython owns the
event loop, and the simulator calls into Python on every callback. Rivet
puts it on top. Rivet's native executor owns the scheduler, a trigger is a
simulator callback, and a Python coroutine is one more task on it.

Concretely, when a testbench writes `await clk.rising_edge()`:

- the trigger object is a Rust enum, not a Python object with a lifecycle;
- registering and deregistering it is a VPI call from Rust;
- the simulator's callback wakes a Rust waker and runs the Rust executor;
- **once** per `await`, that executor resumes the Python coroutine.

That last step is the cost, and it is the only one. cocotb pays it too,
and then pays for a Python scheduler, Python trigger objects, and values
marshalled as decimal strings on top.

## What it costs, measured

Same design, same simulator, same machine, same benchmarks — three
implementations of one testbench in
[`examples/bench`](../examples/bench): Rust on Rivet
([`src/lib.rs`](../examples/bench/src/lib.rs)), Python on Rivet
([`python/test_bench.py`](../examples/bench/python/test_bench.py)), and
Python on cocotb
([`cocotb/test_bench.py`](../examples/bench/cocotb/test_bench.py)).

Icarus Verilog 12.0, 20 000 cycles, median of 3 runs with the range in
brackets, µs of wall-clock per simulated clock cycle:

| benchmark | cocotb 2.1 | Rivet (Python) | Rivet (Rust) | Python vs cocotb |
|---|---|---|---|---|
| `edge_roundtrip` — await an edge every cycle | 23.49 (23.11–24.56) | 3.02 (2.87–4.39) | 2.24 (2.16–2.28) | **7.8×** |
| `value_traffic` — edge, write 32-bit, read 32-bit and 512-bit | 267.01 (266.36–269.03) | 8.26 (8.06–9.03) | 6.27 (6.18–6.56) | **32.3×** |
| `edge_then_readonly` — edge, write, `ReadOnly`, read | 37.39 (36.22–38.01) | 5.47 (5.39–5.55) | 3.48 (3.37–3.53) | **6.8×** |
| `many_tasks` — 100 tasks awaiting every edge | 293.95 (290.72–310.82) | 84.88 (84.81–86.12) | 20.61 (19.64–20.83) | **3.5×** |

Shapes cocotb has no equivalent for:

| benchmark | Rivet (Python) | Rivet (Rust) |
|---|---|---|
| `timer_only` — no clock, no value-change callbacks | 0.95 (0.93–1.06) | 0.26 (0.24–0.34) |
| `clock_only` — clock running, nobody awaiting it | 1.86 (1.52–1.94) | 1.79 (1.75–2.04) |
| `batched_edges` — the same N cycles in one `await` | 2.20 (1.91–2.21) | — |

Reproduce it:

```sh
cargo build --release -p rivet-hdl-cli
pip install cocotb                      # for the cocotb column
ci/bench-python.py --repeat 3 --cycles 20000
```

### Reading that table honestly

**Python costs about 1.3× to 1.6× over Rust on the ordinary shapes**, and
about 4× on `many_tasks`. That is the price of the interpreter, and it is
real. If you want the last of it, the Rust API is still there and the two
run side by side.

**`many_tasks` is the worst case and it is worth understanding.** A
hundred monitor tasks each awaiting every edge is a hundred coroutine
resumes per cycle. Rivet-Python is still 3.5× faster than cocotb, but the
gap to Rust is 4×, because the part that does not scale is exactly the
part that is still Python.

**`clock_only` is the best case and it is the more interesting one.**
Python and Rust are the same speed, because `rivet.Clock` is a native
task: no Python runs per edge, however many cycles the test takes.
Everything that works this way — clocks, the kit, `await
clk.rising_edge(n=...)` — costs a Python testbench nothing at all.

**`batched_edges` is the lesson.** Twenty thousand cycles skipped in one
`await` costs 2.20 µs per cycle against `edge_roundtrip`'s 3.02 — it lands
where Rust does, because it enters the interpreter once instead of twenty
thousand times.

**These are harness-overhead micro-benchmarks and they flatter everyone.**
On a real design, the simulator's own work dominates: the PicoRV32
measurement in [`benchmarks.md`](benchmarks.md) costs 15.27 µs per cycle
with no harness at all. Against that, the difference between 2.24 and 3.02
µs of harness is a few per cent of the run, not 35%.

### Writing a fast testbench

In rough order of how much it matters:

1. **Do not write a per-cycle Python loop you do not need.** `await
   clk.rising_edge(n=1000)` waits a thousand cycles for the price of one.
2. **Let native code run the repetitive parts.** `rivet.Clock` is a Rust
   task. So are `Memory`, `Scoreboard`, `Covergroup` and `Rng` — a
   scoreboard comparison is not a Python comparison.
3. **Hoist handles out of loops.** `dut.signal("clk")` is a hierarchy
   lookup; do it once, not every cycle.
4. **Read as an integer.** `sig.get()` returns an `int` and never builds a
   string. `get_binstr()` does, so keep it for diagnostics.
5. **Prefer one task that watches several signals** over several tasks
   that each watch one, when the difference does not matter — see
   `many_tasks`.

## The API

Everything is on the `rivet` module.

### Tests

```python
@rivet.test                                   # bare
@rivet.test(timeout="100us")                  # simulated-time limit
@rivet.test(skip=True)
@rivet.test(expect_fail=True)
@rivet.test(expect_fail_msg="off by one")     # and for this reason
@rivet.test(expect_timeout=True)
@rivet.test(stage=1)                          # ordering
@rivet.test(wall_timeout=30.0)                # seconds, not simulated
@rivet.test(name="a_better_name")
@rivet.test(param_sets=["w16"])               # only under this parameter set
```

The decorated function is returned unchanged, so it stays importable and
callable like any other.

### The design

```python
clk   = dut.signal("clk")          # AttributeError, naming the scope, if absent
core  = dut.module("u_core")
res   = dut.at("u_core.alu.result")   # dotted path, `mem[3]` indices allowed
sub   = dut.scope_at("u_core.alu")
item  = dut["clk"]                    # for names built at run time
dut.has("clk"), dut.children(), dut.name, dut.path
```

### Values

```python
sig.get()          # int; raises ValueError if any bit is X or Z
sig.get_lossy()    # int, X and Z read as 0
sig.get_signed()   # two's complement
sig.get_binstr()   # "1010xxzz"
sig.get_hexstr()
sig.get_vec()      # a four-state LogicVec
sig.is_resolvable()

sig.set(0xA5)      # inertial deposit: lands at the next evaluation
sig.set_now(0xA5)  # immediate: visible to the next read
sig.force(0xA5); sig.release()
sig.slice(7, 4).set(0x3)       # successive slice writes compose
sig.index(3); sig.member("valid")
sig.width, sig.path, sig.is_const, sig.enum_name()
```

Integers of any width work, including past 64 and 128 bits, and never go
through a string. Negative values are taken as two's complement of the
signal's width. Strings accept Verilog literals (`"8'hA5"`, `"4'b10xz"`),
bare four-state digits (`"1010xxzz"`) and Python prefixes (`"0xA5"`,
`"0b1010"`, `"0o777"`).

One thing to know: a bare string shorter than the signal is
zero-extended, exactly as in the Rust API. `"x"` on an eight-bit signal is
one X bit above seven zeros. Write `"xxxxxxxx"` or `"8'hxx"` for eight.

### Triggers

```python
await clk.rising_edge()           # and falling_edge(), value_change()
await clk.rising_edge(n=1000)     # n edges, one coroutine resume
await rivet.timer("10ns")         # or rivet.timer(10, "ns")
await rivet.read_write()          # values settled, writes still allowed
await rivet.read_only()           # end of time step, no writes
await rivet.next_time_step()
await rivet.yield_now()           # let other tasks run, no time passes
which = await rivet.first(a, b)   # index of the winner; the rest are dropped
```

### Tasks and clocks

```python
task = rivet.start_soon(monitor())        # runs alongside
value = await task.join()                 # or: await task
task.cancel(); task.done; task.cancelled

value = await rivet.with_timeout(coro(), "1us")   # raises TimeoutError

clock = rivet.Clock(clk, "10ns").start()
rivet.Clock(clk, "10ns", high_time="2ns", phase="1ns", jitter="100ps").start()
clock.stop()
```

An exception escaping a started task fails the test, as in cocotb. Pass
`propagate=False` when you mean to handle it at the `join()`.

### Ending a test

```python
rivet.fail("the scoreboard is not empty")   # raises
rivet.skip("no DDR model in this build")    # raises
rivet.finish("seen enough")                 # raises; the test passed
rivet.finish_now()                          # the same, from a background task
```

### The kit

`rivet.Memory`, `rivet.Rng`, `rivet.Scoreboard`, `rivet.Covergroup`,
`rivet.Bins`. These are the same Rust implementations the Rust testbenches
use, so a Python test and a Rust one score against the same scoreboard and
merge into the same coverage database. See [the kit
chapter](book/src/kit.md).

### The run

```python
rivet.now(), rivet.now("ns"), rivet.now_str()
rivet.precision(), rivet.simulator()
rivet.test_seed(), rivet.base_seed()
rivet.dump_tasks()                # every live task and what it awaits
rivet.log.info("counted to %d", n)
```

`rivet.log` goes through Rivet's logger, so Python lines and harness lines
interleave in one stream with one set of timestamps.

## Running

`rivet.toml` gains a `[python]` section:

```toml
[design]
top = "counter"
sources = ["counter.v"]

[python]
tests = ["test_counter"]    # modules to import; their decorators register the tests
paths = ["tb"]              # extra sys.path entries (the manifest's directory is always on it)

[sim.icarus]
args = ["-g2012"]
```

```sh
rivet run --python --sim icarus
rivet run --python --filter counts --seed 42
rivet run --python -j 8              # shard the regression across processes
```

`--python` is implied when `rivet.toml` has a `[python]` section and there
is no Rust crate to build, so in a Python-only project you can leave it
off. Everything else about `rivet run` is unchanged: `--filter`, `--seed`,
`--waves`, `-j`, parameter sets, `results.xml`, coverage merging.

Icarus, GHDL and NVC are supported. Verilator is not: it has no PLI to
load a plugin through — its harness is a binary that links the verilated
model, so it has to be compiled against the design.

## How it is put together

```
your test.py  ──import──►  rivet  ──►  _rivet  (the bindings)
                                          │
simulator ──dlopen──► librivet_python.so ─┤  embedded CPython
                                          │  Rivet's executor
                                          └► VPI / VHPI
```

The simulator loads one shared object. It starts an interpreter with
`_rivet` already in the inittab — so `import rivet` inside the simulation
finds the running simulation's own bindings, not a wheel that knows
nothing about it — installs the VPI or VHPI backend, and at
start-of-simulation imports the modules named in `[python] tests`.
Importing them runs their `@rivet.test` decorators, which is what
registers the tests. From there it is Rivet's ordinary regression loop:
the same seeding, timeouts, per-test waves, `expect_fail` handling and
`results.xml` that `#[rivet::test]` gets, because it is the same code.

Source: [`python/rivet`](../python/rivet). The bindings are
`crates/bridge`, the extension module is `crates/ext`, the PLI plugin is
`crates/plugin`, and the Python package is `src/rivet`.

## Without a simulator

`rivet.mock` is Rivet's own pure-Rust simulator, in process. It is how the
Python layer is tested, and it is useful for trying something out:

```python
import rivet

@rivet.test(timeout="10us")
async def counts(dut):
    clk = dut.signal("clk")
    rivet.Clock(clk, "10ns").start()
    ...

d = rivet.mock.MockDesign("top")
clk   = d.logic("clk", 1)
rst_n = d.logic("rst_n", 1)
en    = d.logic("en", 1)
count = d.logic("count", 8)
d.counter(clk, rst_n, en, count)

for r in d.run():
    print(r["name"], r["status"], r["message"])
```

It schedules, evaluates and delivers callbacks the way a real simulator
does, so everything above the PLI boundary is the same code that runs on
Icarus. What it does not have is your design: the built-in behaviours are
`counter`, `dff` and `adder`.

## Coming from cocotb

The shape is the same — a scheduler, real triggers, `async def` — so the
translation is mechanical. What differs is deliberate: Rivet's Python API
is the Rust API with Python syntax, rather than a second design.

| cocotb | Rivet |
|---|---|
| `@cocotb.test()` | `@rivet.test` or `@rivet.test(...)` |
| `dut.clk` | `dut.signal("clk")` |
| `dut.u_core.alu.result` | `dut.at("u_core.alu.result")` |
| `dut.sig.value` | `sig.get()` |
| `int(dut.sig.value)` / `.to_unsigned()` | `sig.get()` |
| `dut.sig.value = 1` | `sig.set(1)` |
| `dut.sig.setimmediatevalue(1)` | `sig.set_now(1)` |
| `await RisingEdge(dut.clk)` | `await clk.rising_edge()` |
| `await ClockCycles(dut.clk, 10)` | `await clk.rising_edge(n=10)` |
| `await Timer(10, "ns")` | `await rivet.timer("10ns")` |
| `await ReadOnly()` | `await rivet.read_only()` |
| `cocotb.start_soon(coro())` | `rivet.start_soon(coro())` |
| `await First(a, b)` | `i = await rivet.first(a, b)` |
| `Clock(dut.clk, 10, "ns")` + `start_soon` | `rivet.Clock(clk, "10ns").start()` |
| `with_timeout(coro, 1, "us")` | `rivet.with_timeout(coro, "1us")` |
| `raise cocotb.result.TestSuccess` | `rivet.finish()` |
| `dut._log.info(...)` | `rivet.log.info(...)` |

Two differences worth pausing on.

**Handles are explicit.** `dut.signal("clk")` rather than `dut.clk`.
`dut.conut` in cocotb is an `AttributeError` at three in the morning;
`dut.signal("conut")` raises the first time the line runs, naming the
scope and what is in it. It is also the honest cost model — a lookup
walks the hierarchy, so it reads like the call it is, and you hoist it
out of the loop.

**X and Z are not guessed at.** `sig.get()` raises `ValueError` on an
unresolved value rather than returning a number that is silently wrong.
Use `get_lossy()` if you want cocotb's behaviour, or `get_binstr()` to see
what is actually there.

The Rivet-only things worth reaching for once you have ported: the batched
`n=` wait, `rivet.mock`, and the fact that a Rust testbench and a Python
one can share a scoreboard.

## Installing

```sh
pip install rivet-hdl
```

That is the whole thing: the `rivet` package, the bindings, the `rivet`
CLI and the PLI plugin the simulator loads. No cargo, no Rust toolchain.
You still bring your own simulator — Icarus, GHDL or NVC.

```sh
rivet run --python --sim icarus
```

The distribution is `rivet-hdl` because plain `rivet` on PyPI is an
unrelated S3 library; the import stays `rivet`. The Rust crates follow the
same split — `cargo add rivet-hdl` gives you `use rivet::`.

A wheel is specific to a platform *and* a Python minor version, because
the plugin links libpython rather than loading it. (pyo3's `abi3` does not
change that: it narrows the API this code may use, but the build still
links the concrete interpreter. cocotb ships per-version wheels for the
same reason.)

### From a checkout

Nothing to install. `rivet run --python` builds the plugin the first time
and finds the package in the tree:

```sh
git clone https://github.com/A-Ravioli/rivet && cd rivet
cargo build --release -p rivet-hdl-cli
cd ~/my-testbench && ~/rivet/target/release/rivet run --sim icarus
```

### Building a wheel yourself

```sh
cd python/rivet
python3 build-wheel.py --out dist        # for the interpreter running it
```

It builds the CLI and the plugin, stages them in the package, and calls
maturin. `--no-strip` keeps the debug info, which is about 30 MB larger
and worth it when debugging a crash inside the simulator.

Point `rivet run` at a plugin explicitly with `--plugin <path>` or
`RIVET_PYTHON_PLUGIN`. For NVC, build it with `--no-default-features
--features vhpi`: VPI and VHPI cannot share one shared object.

Building any of this from source needs a Python development install —
`libpython` and its headers.
