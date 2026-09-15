# Benchmarks

> Writing the testbench in Python instead of Rust has its own measured
> cost, compared against cocotb on the same design and machine:
> [`python.md`](python.md#what-it-costs-measured).


Harness overhead per clock cycle, measured with `examples/bench` (Rivet) and
`examples/bench/cocotb` (cocotb 2.1.0), 100 000 cycles, one run each, on the
same container. Verilator 5.020 and Icarus Verilog 12.0. Times are wall-clock
per simulated clock cycle of the design in `examples/bench/hdl/bench.sv`
(a 32-bit registered incrementer plus a 512-bit shift register). Rivet built
with `--release`.

Reproduce with:

```sh
cargo build --release -p rivet-cli
RIVET_BENCH_N=100000 target/release/rivet run --sim icarus    --release -C examples/bench
RIVET_BENCH_N=100000 target/release/rivet run --sim verilator --release -C examples/bench
RIVET_BENCH_N=100000 python3 examples/bench/cocotb/run.py icarus
```

## Icarus Verilog 12.0

| Test | What it measures | cocotb 2.1 | Rivet | Ratio |
|---|---|---|---|---|
| `edge_roundtrip` | one task, `await RisingEdge(clk)` per cycle, harness-driven clock | 26.6 µs | 2.50 µs | 10.6× |
| `edge_then_readonly` | edge, write `din`, `await ReadOnly()`, read `dout` | 42.0 µs | 3.96 µs | 10.6× |
| `value_traffic` | edge, write 32-bit, read 32-bit and 512-bit | 305 µs | 7.68 µs | 40× |
| `many_tasks` | 100 tasks each awaiting every edge (per cycle) | 311 µs | 20.8 µs | 15× |

The bare simulator, running the same design from a pure-Verilog testbench
with an `always #5 clk` and no harness at all, takes 1.36 µs per cycle
(`vvp` on a 100 000-cycle `floor_tb`). So the harness overhead on
`edge_roundtrip` is about 1.1 µs for Rivet versus about 25 µs for cocotb.

cocotb's `value_traffic` number is dominated by converting the 512-bit
`LogicArray` to an integer, which goes through a Python string
(`docs/design/00-cocotb-analysis.md` §4.2); Rivet reads the vector straight
into a reusable `aval`/`bval` buffer through `vpiVectorVal`.

## Verilator 5.036 (cocotb baseline)

cocotb 2.x needs Verilator 5.036+ (`doInertialPuts`/`evalNeeded` in its
`verilator.cpp`). Verilator 5.036 was built from source for this
comparison; both harnesses ran the same design and testbench logic.

| Test | cocotb 2.1 | Rivet | Ratio |
|---|---|---|---|
| `edge_roundtrip` | 9.34 µs | 1.89 µs | 4.9× |
| `edge_then_readonly` | 25.9 µs | 2.62 µs | 9.9× |
| `value_traffic` | 371 µs | 2.81 µs | 132× |
| `many_tasks` | 366 µs | 20.2 µs | 18× |

## Verilator 5.020 (Rivet only)

| Test | Rivet |
|---|---|
| `timer_only` (one `Timer` per cycle, no clock) | 0.33 µs |
| `clock_only` (clock task running, nothing awaiting it) | 0.98 µs |
| `edge_roundtrip` | 1.49 µs |
| `edge_roundtrip_immediate_clock` (clock with `NoDelay` writes, no ReadWrite flush) | 1.21 µs |
| `edge_then_readonly` | 1.94 µs |
| `value_traffic` | 1.76 µs |
| `many_tasks` (100 tasks, per cycle) | 18.9 µs |

The model is compiled with `-O2` in release builds (Verilator's default is
`-Os`), which is worth 10 to 18% by itself. Values are read and written
directly in the model's storage (`VerilatedScope::varFind`), bypassing
Verilator's VPI for everything except value-change detection; set
`RIVET_VERILATOR_DIRECT=0` to compare against the VPI path.

Each simulator event delivered to the harness (a timer, a value change, a
phase callback) costs about 0.3 µs on Verilator, including the model
evaluation that follows it. What got it here, in order: native timer and
phase scheduling instead of VPI registrations (2.7 to 2.0 µs on
`edge_roundtrip`), `-O2` model builds, direct value access, and replacing
SipHash maps and per-event allocations in the runtime after a callgrind
profile showed hashing at 15% and malloc/free at 11% of instructions.
Remaining known costs: two `eval_step` calls per half period (after the
timer and after the ReadWrite flush), Verilator's `callValueCbs` scan, and
the thread-local runtime borrow on every call into the runtime.


## Median and spread, and a real design

The tables above are single runs. `ci/bench.py` repeats each benchmark and
reports the median with the spread, and the nightly `soak` workflow runs it
against `bench-baseline.json` so a regression fails a build rather than
being noticed months later.

`examples/bench_soc` is a real design: PicoRV32 (ISC licensed, vendored)
running a two-instruction loop out of a memory, with the core's memory
interface visible to the testbench. It answers the question the tables
above cannot: what fraction of a real simulation the harness costs.

Icarus Verilog 12, 100 000 cycles, five runs, release harness:

| Benchmark | Median µs/cycle | Spread |
|---|---|---|
| `timer_only` | 0.19 | 26% |
| `edge_roundtrip_immediate_clock` | 1.50 | 13% |
| `clock_only` | 1.67 | 6% |
| `edge_roundtrip` | 2.13 | 9% |
| `edge_then_readonly` | 3.08 | 22% |
| `value_traffic` | 4.97 | 27% |
| `soc_clock_only` (PicoRV32) | 19.99 | 12% |
| `soc_with_monitor` (PicoRV32) | 19.77 | 12% |

The same PicoRV32 simulation driven by a pure-Verilog testbench, with no
harness at all, takes 15.27 µs/cycle. So on a design that is doing real
work the harness costs about 4.7 µs per cycle, which is roughly a quarter
of the run, and a monitor task watching the memory interface every cycle
adds nothing measurable on top.

Reproduce with:

```sh
cargo build --release -p rivet-cli
ci/bench.py --repeat 5 --cycles 100000 --release
ci/bench.py --repeat 5 --cycles 100000 --release --baseline docs/bench-baseline.json
```

Spreads above 20% on the shortest benchmarks are the container's
scheduling noise, not the harness: `timer_only` is a fifth of a
microsecond per cycle, where a single descheduling event moves the number.

## Edit to result

What a testbench author actually waits for, measured by `ci/loop-latency.py`
on `examples/dff` (one test selected, so the number is the loop and not the
suite):

| Edit | Icarus | Verilator |
|---|---|---|
| nothing changed | 0.1 s | 0.1 s |
| one line in the test crate | 0.4 s | 0.6 s |
| one line in the HDL | 0.4 s | 12.5 s |
| everything, from clean | 0.8 s | 13.0 s |

The Rust rebuild is not the cost it was assumed to be: a test-only edit is
under a second on both simulators, because cargo rebuilds one small crate
and relinks. What costs twelve seconds is Verilator compiling the design
again after an HDL edit, which is Verilator's own work and would not be
helped by anything Rivet does differently. `rivet watch` keeps the loop to
the numbers above without retyping the command.

Python has no compile step at all, so cocotb's test-edit loop is shorter
than 0.4 s. That gap is real; it is a fraction of a second, not the several
seconds it was assumed to be, which is why Rivet does not load the test
library dynamically to shave it.

Reproduce with:

```sh
ci/loop-latency.py --sim icarus --example dff --filter counter
ci/loop-latency.py --sim verilator --example dff --filter counter
```

## What these numbers are not

- The cocotb comparison tables are one run each; the Rivet numbers above
  are medians of five. Expect ±10% on the comparison tables.
- Debug-mode Rivet is roughly 3× slower than these release numbers.
- A different design, or a testbench that does real work per cycle, will be
  dominated by other costs. The point of the table is the harness floor.
