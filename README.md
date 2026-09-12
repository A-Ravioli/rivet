# rivet

Faster cocotb-style hardware verification in Rust.

Rivet drives HDL simulators from Rust `async` testbenches. It keeps cocotb's
timing model and vocabulary (`RisingEdge`, `ReadWrite`, `ReadOnly`, `Timer`,
`Deposit`/`Force`/`Release`, `start_soon`) and replaces the embedded Python
interpreter and string-typed values with an in-process native executor and
`aval`/`bval` vectors. On Icarus the harness overhead per clock edge is about
1.4 µs against roughly 25 µs for cocotb; see [`docs/benchmarks.md`](docs/benchmarks.md).

Status: Icarus Verilog, Verilator, and GHDL (VHDL) work end to end; see
[`docs/design/03-status.md`](docs/design/03-status.md). Design documents are in
[`docs/design/`](docs/design/README.md).

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
target/debug/rivet run --sim icarus    -C examples/dff
target/debug/rivet run --sim verilator -C examples/dff
target/debug/rivet run --sim ghdl      -C examples/dff_vhdl
target/debug/rivet run --sim icarus -C examples/dff --filter counter --waves --log debug
```

`rivet run` builds the crate, compiles the design described in `rivet.toml`,
loads the harness into the simulator, and reads back a cocotb-compatible
`results.xml` from `sim_build/<sim>/`.

## Layout

| Crate | Role |
|---|---|
| `rivet-core` | executor, triggers, values, handles, `Backend` trait, runtime, test registry |
| `rivet-mock` | pure-Rust event simulator for testing the harness itself |
| `rivet-vpi` | VPI backend (Icarus, Verilator's VPI, and the cocotb-catalogued quirks for others) |
| `rivet-verilator` | Verilator build helper, C++ shim, simulation main loop |
| `rivet-macros` | `#[rivet::test]` |
| `rivet-kit` | `reset`, `Driver`/`Monitor`, valid/ready handshake, `Scoreboard` |
| `rivet-manifest` | `rivet.toml` |
| `rivet-cli` | the `rivet` command |
| `rivet` | facade crate |

## Requirements

Rust stable, and Icarus Verilog 11+, Verilator 5.x with a C++17 compiler, or GHDL 4.x.
