# Benchmarks

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
| `edge_roundtrip` | one task, `await RisingEdge(clk)` per cycle, harness-driven clock | 26.6 µs | 2.77 µs | 9.6× |
| `edge_then_readonly` | edge, write `din`, `await ReadOnly()`, read `dout` | 42.0 µs | 4.38 µs | 9.6× |
| `value_traffic` | edge, write 32-bit, read 32-bit and 512-bit | 305 µs | 7.30 µs | 42× |
| `many_tasks` | 100 tasks each awaiting every edge (per cycle) | 311 µs | 23.2 µs | 13× |

The bare simulator, running the same design from a pure-Verilog testbench
with an `always #5 clk` and no harness at all, takes 1.36 µs per cycle
(`vvp` on a 100 000-cycle `floor_tb`). So the harness overhead on
`edge_roundtrip` is about 1.4 µs for Rivet versus about 25 µs for cocotb.

cocotb's `value_traffic` number is dominated by converting the 512-bit
`LogicArray` to an integer, which goes through a Python string
(`docs/design/00-cocotb-analysis.md` §4.2); Rivet reads the vector straight
into a reusable `aval`/`bval` buffer through `vpiVectorVal`.

## Verilator 5.020 (Rivet only)

cocotb 2.x's Verilator support requires Verilator 5.036 or newer
(`doInertialPuts`/`evalNeeded` in its `verilator.cpp`), which this container
does not have, so there is no cocotb baseline here yet.

| Test | Rivet |
|---|---|
| `timer_only` (one `Timer` per cycle, no clock) | 0.46 µs |
| `clock_only` (clock task running, nothing awaiting it) | 1.36 µs |
| `edge_roundtrip` | 2.01 µs |
| `edge_roundtrip_immediate_clock` (clock with `NoDelay` writes, no ReadWrite flush) | 1.61 µs |
| `edge_then_readonly` | 3.17 µs |
| `value_traffic` | 2.69 µs |
| `many_tasks` (100 tasks, per cycle) | 23.7 µs |

Each simulator event delivered to the harness (a timer, a value change, a
phase callback) currently costs about 0.4 to 0.5 µs on Verilator, including
the model evaluation that follows it. The Verilator backend already schedules
timers and phase callbacks natively (`rivet-verilator/src/sched.rs`) instead
of through VPI registrations, which took `edge_roundtrip` from 2.7 µs to
2.0 µs. Remaining known costs, in likely order: two `eval_step` calls per
half period (one after the timer, one after the ReadWrite flush), the
per-event `HashMap` and `Box` traffic in the runtime, and the `Arc`-based
wakers. Direct signal access without VPI (roadmap M5) is the next big step.

## What these numbers are not

- One run each, no statistical treatment; expect ±10%.
- Debug-mode Rivet is roughly 3× slower than these release numbers.
- A different design, or a testbench that does real work per cycle, will be
  dominated by other costs. The point of the table is the harness floor.
