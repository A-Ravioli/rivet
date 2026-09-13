# rivet

Faster cocotb-style hardware verification in Rust.

Rivet drives HDL simulators from Rust `async` testbenches. It keeps cocotb's
timing model and vocabulary (`RisingEdge`, `ReadWrite`, `ReadOnly`, `Timer`,
`Deposit`/`Force`/`Release`, `start_soon`) and replaces the embedded Python
interpreter and string-typed values with an in-process native executor and
`aval`/`bval` vectors. On Icarus the harness overhead per clock edge is about
1.4 µs against roughly 25 µs for cocotb; see [`docs/benchmarks.md`](docs/benchmarks.md).

Status: four simulators are verified end to end. Icarus Verilog 12 and
Verilator 5.020/5.036 through VPI and direct access, NVC 1.23 through VHPI,
GHDL 4.1 through its VPI. Questa, Xcelium, VCS, Riviera and DSim have launch
flows but have never been run; see
[`docs/design/03-status.md`](docs/design/03-status.md) and
[`docs/SIMULATOR-QUIRKS.md`](docs/SIMULATOR-QUIRKS.md). The book is in
[`docs/book`](docs/book), design documents in
[`docs/design/`](docs/design/README.md); coming from cocotb, start with
[`docs/migration.md`](docs/migration.md).

What it does beyond cocotb:

- Seeded random stimulus (`rivet::rng()`, `#[derive(Randomize)]` with
  constraints); every run prints its seed and `--seed` replays it.
- Functional coverage (`Covergroup`, `Bins`, crosses), merged across runs,
  `rivet cov report --threshold 90`.
- Bus models: AXI4-Lite, AXI4, AXI4-Stream, APB, Avalon-MM, Wishbone
  masters and memory-backed slaves with backpressure; a sparse `Memory`
  with `$readmemh` loading; reference models feeding a scoreboard.
- Checkers (`assert_never`, `assert_implies`, `assert_no_x`, ...), golden
  transaction traces (`assert_trace!`), hang diagnostics that list every
  task and its trigger, per-test wall-clock limits.
- `rivet run -j 8` shards tests across simulator processes; `rivet watch`
  reruns on change; `[design.param_sets]` rebuilds the design per
  parameterisation; `#[rivet::test(params = [8, 16])]`.
- `rivet bindgen` turns `typedef enum` and `typedef struct packed` into
  Rust types; per-test log files and JSON logs; per-test waveform files.
- `rivet new counter` scaffolds a testbench crate that passes as generated.
- `#[rivet::fixture]` for async setup a test asks for by argument name, torn
  down when the value drops; `rivet::skip(reason)` and
  `rivet::runtime::finish_test()` end a test from anywhere.
- Regular-expression filters, `--shuffle`, `expect_fail = "message"`,
  `expect_timeout`, and `file`/`lineno` in `results.xml`; `cargo nextest run`
  puts each test in its own process.
- `Signal::slice(hi, lo)` for bit ranges; `enum_name()` for VHDL enumeration
  literals where the simulator reports them.
- `fusesoc run --tool rivet` through the Edalize backend in
  `integrations/edalize`; `python/rivet_py` brings the kit to cocotb.

## A testbench

```rust
use rivet::prelude::*;

#[rivet::test(timeout = 10.us())]
async fn counter_counts(dut: Module) -> rivet::Result<()> {
    let clk = dut.signal("clk")?;
    let _clock = Clock::start(clk, 10.ns());
    dut.signal("rst_n")?.set(0);
    clk.rising_edge().await;
    dut.signal("rst_n")?.set(1);
    for _ in 0..4 {
        clk.rising_edge().await;
    }
    read_only().await;
    assert_eq!(dut.signal("count")?.get_u64()?, 4);
    Ok(())
}
```

A test crate is a `cdylib` (loaded by PLI simulators) and, for Verilator, a
binary that links the model. See [`examples/dff`](examples/dff) for the
`Cargo.toml`, `build.rs`, `rivet.toml`, and `main.rs` it needs.

## Running

```sh
cargo build -p rivet-cli
target/debug/rivet new counter --path "$PWD"   # a crate whose 2 tests pass as generated
target/debug/rivet run --sim icarus -C counter

target/debug/rivet run --sim icarus    -C examples/dff
target/debug/rivet run --sim verilator -C examples/dff
target/debug/rivet run --sim ghdl      -C examples/dff_vhdl
target/debug/rivet run --sim nvc       -C examples/vhdl_types
target/debug/rivet run --sim icarus -C examples/dff --filter counter --waves --log debug
target/debug/rivet run --sim icarus -C examples/bus -j 4 --cov-threshold 95 --seed 42
target/debug/rivet watch --sim verilator -C examples/bus
target/debug/rivet cov report -C examples/bus --threshold 100
```

`--path` points the new crate at this checkout; without it the crate depends
on the published `rivet` crate. `rivet run` builds the crate, compiles the
design described in `rivet.toml`, loads the harness into the simulator, and
reads back a cocotb-compatible `results.xml` from `sim_build/<sim>/`.

With the `harness` feature and a `harness = false` test target (see
`examples/dff/tests/sim.rs`), plain `cargo test` works too:

```sh
cargo test -p example-dff -- --list        # no simulator needed
cargo test -p example-dff                  # runs on Icarus
RIVET_SIM=verilator cargo test -p example-dff -- counter
```

## Typed bindings

```sh
target/debug/rivet bindgen --sim icarus -C examples/dff   # writes src/dut.rs
```

generates a struct per module with a field per signal, so a misspelled
signal is a compile error and a width change is caught when the test binds.
`typedef enum` and `typedef struct packed` declarations in the sources
become Rust enums and structs with `from_signal` and `set_on`:

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

## Layout

| Crate | Role |
|---|---|
| `rivet-core` | executor, triggers, values, handles, `Backend` trait, runtime, test registry |
| `rivet-mock` | pure-Rust event simulator for testing the harness itself |
| `rivet-vpi` | VPI backend (Icarus, GHDL, Verilator's VPI, and the cocotb-catalogued quirks for others) |
| `rivet-vhpi` | VHPI backend (NVC; Questa and Riviera unverified) |
| `rivet-verilator` | Verilator build helper, C++ shim, simulation main loop |
| `rivet-macros` | `#[rivet::test]`, `#[rivet::fixture]`, `#[derive(Randomize)]` |
| `rivet-kit` | `Reset`, `Driver`/`Monitor`, bus models, `Memory`, `Scoreboard`, `ModelScoreboard`, checkers, `Trace` |
| `rivet-manifest` | `rivet.toml` |
| `rivet-cli` | the `rivet` command |
| `rivet` | facade crate |
| `integrations/edalize` | Edalize tool backend (`fusesoc run --tool rivet`) |
| `python/rivet_py` | Python bindings to the kit for cocotb testbenches |

## Requirements

Rust 1.87 or later, and one of Icarus Verilog 11+, Verilator 5.x with a
C++17 compiler, GHDL 4.x or NVC 1.17+ for VHDL.
