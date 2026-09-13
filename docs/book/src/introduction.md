# Introduction

Rivet is a hardware verification harness. It drives HDL simulators from Rust
`async` testbenches. It keeps cocotb's timing model and vocabulary
(`RisingEdge`, `ReadWrite`, `ReadOnly`, `Timer`, `Deposit`/`Force`/`Release`,
`start_soon`) and replaces the embedded Python interpreter and the
string-typed value path with an in-process native executor and `aval`/`bval`
vectors.

A test looks like this. The example is `examples/dff/src/lib.rs`.

```rust
use rivet::prelude::*;

#[rivet::test]
async fn counter_counts(dut: Module) -> rivet::Result<()> {
    let clk = dut.signal("clk")?;
    let count = dut.signal("count")?;
    let _clock = Clock::start(clk, 10.ns());
    reset(&dut, clk).await?;
    read_only().await;
    let base = count.get_u64()?;
    for i in 1..=20u64 {
        clk.rising_edge().await;
        read_only().await;
        assert_eq!(count.get_u64()?, base + i);
    }
    Ok(())
}
```

`rivet run` builds the test crate, compiles the design described in
`rivet.toml`, loads the harness into the simulator, and writes a
cocotb-compatible `results.xml` into `sim_build/<sim>/`.

## How Rivet differs from cocotb

| Area | cocotb | Rivet |
|---|---|---|
| Testbench language | Python, interpreter embedded in the simulator | Rust, compiled into a `cdylib` (PLI simulators) or a binary (Verilator) |
| Concurrency | coroutines on cocotb's scheduler | `async` futures on a single-threaded, FIFO, deterministic executor |
| Values | binary strings through `LogicArray` | four-state `aval`/`bval` vectors read and written with `vpiVectorVal` |
| Integer writes | `set_signal_val_int` is 32-bit; wider values become formatted strings | `u64`, `u128` and big integers go straight into the vector encoding |
| Signal access | attribute lookup (`dut.sig`) | `dut.signal("sig")?`, plus typed bindings from `rivet bindgen` |
| Build | Makefile or `cocotb_tools.runner` | `rivet.toml` plus a Cargo crate; content-hashed rebuilds |
| Regression runner | `RegressionManager` | the same loop, plus `cargo test` through `rivet::harness::main()` |
| Random stimulus | `random` with `RANDOM_SEED` | `rivet::rng()` per-test streams, `#[derive(Randomize)]` with constraints |
| Functional coverage | the separate `cocotb-coverage` package | `Covergroup`, `Bins`, `Cross` in the core, merged across runs |
| Bus models | the separate `cocotb-bus` and `cocotbext-*` packages | `rivet-kit`: AXI4-Lite, AXI4, AXI4-Stream, APB, Avalon-MM, Wishbone |
| Hang diagnostics | none | timeouts print every live task and the trigger it waits on |
| Parallelism | pytest-xdist with separate build directories | `rivet run -j N` shards tests across simulator processes in one build |

The timing model is deliberately the same. Five phases per time step, the
same triggers legal in each phase, and the same illegal transitions. See
[The timing model](timing-model.md).

## Benchmarks

Harness overhead per clock cycle, measured with `examples/bench` (Rivet) and
`examples/bench/cocotb` (cocotb 2.1.0), 100 000 cycles, one run each, on the
same container. Times are wall-clock per simulated clock cycle of the design
in `examples/bench/hdl/bench.sv`, a 32-bit registered incrementer plus a
512-bit shift register. Rivet built with `--release`.

### Icarus Verilog 12.0

| Test | What it measures | cocotb 2.1 | Rivet | Ratio |
|---|---|---|---|---|
| `edge_roundtrip` | one task, `await RisingEdge(clk)` per cycle, harness-driven clock | 26.6 µs | 2.50 µs | 10.6× |
| `edge_then_readonly` | edge, write `din`, `await ReadOnly()`, read `dout` | 42.0 µs | 3.96 µs | 10.6× |
| `value_traffic` | edge, write 32-bit, read 32-bit and 512-bit | 305 µs | 7.68 µs | 40× |
| `many_tasks` | 100 tasks each awaiting every edge (per cycle) | 311 µs | 20.8 µs | 15× |

The bare simulator, running the same design from a pure-Verilog testbench
with an `always #5 clk` and no harness at all, takes 1.36 µs per cycle. So
the harness overhead on `edge_roundtrip` is about 1.1 µs for Rivet against
about 25 µs for cocotb.

### Verilator 5.036

cocotb 2.x needs Verilator 5.036 or later. Verilator 5.036 was built from
source for this comparison; both harnesses ran the same design and testbench
logic.

| Test | cocotb 2.1 | Rivet | Ratio |
|---|---|---|---|
| `edge_roundtrip` | 9.34 µs | 1.89 µs | 4.9× |
| `edge_then_readonly` | 25.9 µs | 2.62 µs | 9.9× |
| `value_traffic` | 371 µs | 2.81 µs | 132× |
| `many_tasks` | 366 µs | 20.2 µs | 18× |

### Verilator 5.020, Rivet only

| Test | Rivet |
|---|---|
| `timer_only` (one `Timer` per cycle, no clock) | 0.33 µs |
| `clock_only` (clock task running, nothing awaiting it) | 0.98 µs |
| `edge_roundtrip` | 1.49 µs |
| `edge_roundtrip_immediate_clock` (clock with `NoDelay` writes, no ReadWrite flush) | 1.21 µs |
| `edge_then_readonly` | 1.94 µs |
| `value_traffic` | 1.76 µs |
| `many_tasks` (100 tasks, per cycle) | 18.9 µs |

What these numbers are not: one run each, no statistical treatment, expect
±10%. A debug build of Rivet is roughly 3× slower than these release
numbers. A testbench that does real work per cycle will be dominated by
other costs. The point of the table is the harness floor.

Reproduce with:

```sh
cargo build --release -p rivet-cli
RIVET_BENCH_N=100000 target/release/rivet run --sim icarus    --release -C examples/bench
RIVET_BENCH_N=100000 target/release/rivet run --sim verilator --release -C examples/bench
RIVET_BENCH_N=100000 python3 examples/bench/cocotb/run.py icarus
```

## Status

Icarus Verilog, Verilator and GHDL work end to end. Verified runs are listed
in `docs/design/03-status.md`:

| Example | Tests | Simulators |
|---|---|---|
| `examples/dff` | 10 | Icarus 12.0, Verilator 5.020 and 5.036 |
| `examples/fifo` | 3 | Icarus, Verilator |
| `examples/conformance` | 21 per simulator | Icarus, Verilator 5.020 and 5.036 |
| `examples/bus` | 13 tests, 25 results over two parameter sets | Icarus, Verilator 5.020 and 5.036 |
| `examples/dff_vhdl` | 2 | GHDL 4.1 |
| `examples/bench` | 7 | Icarus, Verilator |

Commercial simulators (Questa, Xcelium, VCS, Riviera) have quirk handling in
the VPI backend but have never been run. There is no VHPI backend, so NVC is
not supported yet; GHDL runs through its VPI. See
[Simulators](simulators.md).

## Crate layout

| Crate | Role |
|---|---|
| `rivet-core` | executor, triggers, values, handles, `Backend` trait, runtime, test registry |
| `rivet-mock` | pure-Rust event simulator for testing the harness itself |
| `rivet-vpi` | VPI backend (Icarus, Verilator's VPI, and cocotb's catalogued quirks for others) |
| `rivet-verilator` | Verilator build helper, C++ shim, simulation main loop |
| `rivet-macros` | `#[rivet::test]`, `#[derive(Randomize)]` |
| `rivet-kit` | `Reset`, `Driver`/`Monitor`, bus models, `Memory`, `Scoreboard`, `ModelScoreboard`, checkers, `Trace` |
| `rivet-manifest` | `rivet.toml` |
| `rivet-cli` | the `rivet` command |
| `rivet` | facade crate |
| `integrations/edalize` | Edalize tool backend (`fusesoc run --tool rivet`) |
| `python/rivet_py` | Python bindings to the kit for cocotb testbenches |
