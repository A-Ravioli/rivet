# Rivet

Cocotb-style hardware verification in Rust: `async` testbenches that drive a
real simulator, without the embedded Python interpreter.

Rivet keeps cocotb's timing model and vocabulary — `RisingEdge`, `ReadWrite`,
`ReadOnly`, `Timer`, `Deposit`/`Force`/`Release`, `start_soon` — and replaces
the Python runtime and its string-typed values with an in-process native
executor and `aval`/`bval` vectors. A test is a normal Rust `async fn`, so
the compiler checks it, `cargo test` runs it, and a misspelled signal can be
a build error rather than a runtime one.

| Per simulated clock cycle, Icarus Verilog 12 | cocotb 2.1 | Rivet |
|---|---|---|
| await an edge each cycle | 26.6 µs | **2.50 µs** |
| edge, write 32-bit, read 32-bit and 512-bit | 305 µs | **7.68 µs** |
| 100 tasks awaiting every edge | 311 µs | **20.8 µs** |

On a real design — PicoRV32 running a program for 100k cycles — the whole
harness costs about 4.7 µs per cycle, roughly a quarter of the run, and a
monitor task watching the memory interface adds nothing measurable. Full
tables, method, and what the numbers are *not*, in
[`docs/benchmarks.md`](docs/benchmarks.md).

## Quick start

```sh
cargo build -p rivet-cli                        # 0.1.0 is unreleased; build from source
target/debug/rivet new counter --path "$PWD"    # a crate whose two tests pass as generated
target/debug/rivet run --sim icarus -C counter
```

`rivet run` builds the test crate, compiles the design described in
`rivet.toml`, loads the harness into the simulator, and reads back a
cocotb-compatible `results.xml` from `sim_build/<sim>/`. `--path` points the
generated crate at this checkout, which is how to use Rivet today: version
0.1.0 has not been published to crates.io yet.

## A test

Two of the tests `rivet new` writes, unedited:

```rust
use rivet::prelude::*;

#[rivet::test(timeout = 100.us())]
async fn counts_when_enabled(dut: Module) -> rivet::Result<()> {
    let clk = dut.signal("clk")?;
    let _clock = Clock::start(clk, 10.ns());
    reset(&dut, clk).await?;

    let count = dut.signal("count")?;
    dut.signal("en")?.set(1);
    for i in 1..=20u64 {
        clk.rising_edge().await;
        // The edge returns before the flop updates; read after it settles.
        read_only().await;
        assert_eq!(count.get_u64()?, i, "cycle {i}");
        next_time_step().await;
    }
    Ok(())
}

#[rivet::test(timeout = 100.us())]
async fn load_overrides_count(dut: Module) -> rivet::Result<()> {
    let clk = dut.signal("clk")?;
    let _clock = Clock::start(clk, 10.ns());
    reset(&dut, clk).await?;

    // A fresh random value every run; `rivet run --seed N` replays it.
    let want = rivet::rng().gen_range(0..=255u64);
    dut.signal("d")?.set(want);
    dut.signal("load")?.set(1);
    clk.rising_edge().await;
    read_only().await;
    assert_eq!(dut.signal("count")?.get_u64()?, want, "loaded value");
    Ok(())
}
```

An edge returns *before* the HDL reacts to it, exactly as in cocotb, so
`read_only().await` is how a test sees the settled value; writes there are
refused rather than silently lost. [`examples/dff`](examples/dff) is a
complete crate — `Cargo.toml`, `build.rs`, `rivet.toml`, and the `cdylib`
entry point the simulator loads.

## Simulator support

| Simulator | Interface | Status |
|---|---|---|
| Icarus Verilog 12 | VPI | verified in CI |
| Verilator 5.020 and 5.036 | direct access, VPI for value changes | verified in CI |
| GHDL 4.1 | VPI | verified in CI |
| NVC 1.17 (developed against 1.23) | VHPI | verified in CI |
| Questa, Xcelium, VCS, Riviera, DSim | VPI / VHPI | launch flows and quirks written, **never run** |

The unverified column is the honest one: the code carries cocotb's
catalogued workaround for each of those tools, but nobody has run it. A
licence holder should start with [`examples/conformance`](examples/conformance).
Per-simulator behaviour, each marked verified or not, is in
[`docs/SIMULATOR-QUIRKS.md`](docs/SIMULATOR-QUIRKS.md). Mixed-language
designs have a routing backend that is tested only against two mock
backends, never a simulator hosting both languages.

## What you get

**Stimulus and coverage.** `rivet::rng()` gives every test its own stream
derived from the run seed and the test name; `#[derive(Randomize)]` covers
ranges, weights, enum variants and rejection-sampled constraints. Every run
prints its seed, each test's seed lands in `results.xml`, and `--seed`
replays it. Functional coverage (`Covergroup`, `Bins`, crosses) merges across
shards and parameter sets, with `rivet cov report --threshold 90`.

**The kit.** AXI4-Lite, AXI4, AXI4-Stream, APB, Avalon-MM and Wishbone
masters, plus memory-backed slaves with configurable backpressure; a sparse
`Memory` with `$readmemh` loading; reference models feeding a `Scoreboard` —
including a Python model, so an existing numpy model can score a Rivet test;
checkers (`assert_never`, `assert_implies`, `assert_no_x`, …); and golden
transaction traces through `assert_trace!`.

**Regressions.** `rivet run -j 8` shards tests across simulator processes and
merges the results and coverage; `[design.param_sets]` rebuilds the design
per parameterisation; `#[rivet::test(params = [8, 16])]` expands one test per
value. Filters are regular expressions, `--shuffle` is reproducible from the
run seed, and `expect_fail = "message"`, `expect_timeout`, `rivet::skip` and
`rivet::runtime::finish_test()` cover the ways a test ends.

**When it goes wrong.** A timeout prints every live task and what it is
waiting on, so a hang names itself instead of just stopping. Per-test
wall-clock limits fail in-band, and a watchdog aborts a process whose
simulator never returns control. Logs are per-test files or JSON;
waveforms can be per-test, or a window opened around the interesting part.

**Tooling.** `rivet bindgen` turns the hierarchy — and any `typedef enum` or
`typedef struct packed` — into Rust types; `rivet watch` reruns on change;
`rivet new` scaffolds a crate; `#[rivet::fixture]` provides async setup a
test asks for by argument name, torn down when the value drops.

**Interop.** `fusesoc run --tool rivet` through the Edalize backend in
[`integrations/edalize`](integrations/edalize), and
[`python/rivet_py`](python/rivet_py), which brings the memory model, RNG,
scoreboard and coverage to testbenches still running on cocotb.

## Running tests

```sh
target/debug/rivet run --sim icarus    -C examples/dff
target/debug/rivet run --sim verilator -C examples/dff
target/debug/rivet run --sim ghdl      -C examples/dff_vhdl
target/debug/rivet run --sim nvc       -C examples/vhdl_types
target/debug/rivet run --sim icarus -C examples/dff --filter counter --waves --log debug
target/debug/rivet run --sim icarus -C examples/bus -j 4 --cov-threshold 95 --seed 42
target/debug/rivet watch --sim verilator -C examples/bus
target/debug/rivet cov report -C examples/bus --threshold 100
```

With the `harness` feature and a `harness = false` test target (see
[`examples/dff/tests/sim.rs`](examples/dff/tests/sim.rs)), plain `cargo test`
and `cargo nextest` work too — nextest gives each test its own process:

```sh
cargo test -p example-dff -- --list        # no simulator needed
cargo test -p example-dff                  # runs on Icarus
RIVET_SIM=verilator cargo test -p example-dff -- counter
cargo nextest run -p example-dff
```

## Typed bindings

```sh
target/debug/rivet bindgen --sim icarus -C examples/dff   # writes src/dut.rs
```

This generates a struct per module with a field per signal, so a misspelled
signal is a compile error and a width change is caught when the test binds.
`typedef enum` and `typedef struct packed` declarations become Rust enums and
structs with `from_signal` and `set_on`:

```rust
mod dut;
use dut::Dut;

#[rivet::test]
async fn typed(dut: Dut) -> rivet::Result<()> {
    let _clock = Clock::start(dut.clk, 10.ns());
    dut.d.set(0x5a);
    dut.clk.rising_edge().await;
    read_only().await;
    assert_eq!(dut.q.get_u64()?, 0x5a);
    Ok(())
}
```

## Documentation

| | |
|---|---|
| [The book](docs/book) | quickstart, timing model, writing tests, the kit, debugging, CLI reference |
| [Migrating from cocotb](docs/migration.md) | equivalence tables, trigger by trigger |
| [Benchmarks](docs/benchmarks.md) | method, medians and spread, a real design, edit-to-result latency |
| [Simulator quirks](docs/SIMULATOR-QUIRKS.md) | every per-simulator behaviour, and whether it was observed or inherited from cocotb |
| [Testing](docs/testing.md) | what is tested where, what is not, and the defects the suites found |
| [Design](docs/design/README.md) | the cocotb analysis this was built from, the architecture, and the status of each milestone |

## Repository layout

| Crate | Role |
|---|---|
| `rivet` | facade: what a testbench imports |
| `rivet-core` | executor, triggers, values, handles, `Backend` trait, runtime, test registry |
| `rivet-macros` | `#[rivet::test]`, `#[rivet::fixture]`, `#[derive(Randomize)]` |
| `rivet-kit` | `Reset`, `Driver`/`Monitor`, bus models, `Memory`, scoreboards, checkers, `Trace` |
| `rivet-vpi` | VPI backend (Icarus, GHDL, and the catalogued quirks for the commercial tools) |
| `rivet-vhpi` | VHPI backend (NVC; Questa and Riviera unverified) |
| `rivet-verilator` | Verilator build helper, C++ shim, simulation main loop |
| `rivet-mock` | pure-Rust event simulator, so the harness can be tested without an EDA tool |
| `rivet-manifest` | `rivet.toml` |
| `rivet-cli` | the `rivet` command |
| `integrations/edalize` | Edalize tool backend (`fusesoc run --tool rivet`) |
| `python/rivet_py` | Python bindings to the kit, for cocotb testbenches |

## Requirements

Rust 1.87 or later (checked in CI), and one of: Icarus Verilog 11+,
Verilator 5.x with a C++17 compiler, GHDL 4.x, or NVC 1.17+.

## Development

```sh
# No EDA tool needed: the harness is tested against a pure-Rust simulator,
# and the CLI's end-to-end tests skip themselves when their tool is missing.
cargo test -p rivet-core -p rivet-mock -p rivet-kit -p rivet-cli
cargo test --workspace          # adds the examples, which need Icarus installed
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all -- --check
```

What each layer covers, and what it deliberately does not, is in
[`docs/testing.md`](docs/testing.md).

## License

MIT or Apache-2.0, at your option.
