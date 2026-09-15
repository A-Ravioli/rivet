# rivet ⚙️

[![CI](https://github.com/A-Ravioli/rivet/actions/workflows/ci.yml/badge.svg)](https://github.com/A-Ravioli/rivet/blob/main/.github/workflows/ci.yml)
[![License](https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-blue)](#license)
[![Rust](https://img.shields.io/badge/rust-1.87%2B-orange)](https://rustup.rs)
[![Simulators](https://img.shields.io/badge/simulators-Icarus%20%7C%20Verilator%20%7C%20GHDL%20%7C%20NVC-informational)](#simulators)

**[Read the book](docs/book/src/introduction.md)** | **[Migrating from cocotb](docs/migration.md)** | **[Benchmarks](docs/benchmarks.md)**

Rivet drives HDL simulators from Rust `async` testbenches. It keeps everything
cocotb got right — the timing model, `RisingEdge`, `ReadWrite`, `ReadOnly`,
`Timer`, `Deposit`/`Force`/`Release` — and throws out the embedded Python
interpreter that makes every one of those cost microseconds.

Your testbench is a regular Rust crate. Signals are typed, not strings. A
misspelled port is a compile error. `cargo test` runs it. And awaiting a clock
edge costs 2.5 µs instead of 26.6 µs.

Rivet runs on Icarus Verilog, Verilator, GHDL and NVC — Verilog and VHDL, VPI
and VHPI — all four verified in CI on every commit, and Icarus on macOS too.

> One of the two tests `rivet new` generates for you, unedited:
>
> ```rust
> use rivet::prelude::*;
>
> #[rivet::test(timeout = 100.us())]
> async fn counts_when_enabled(dut: Module) -> rivet::Result<()> {
>     let clk = dut.signal("clk")?;
>     let _clock = Clock::start(clk, 10.ns());
>     reset(&dut, clk).await?;
>
>     let count = dut.signal("count")?;
>     dut.signal("en")?.set(1);
>     for i in 1..=20u64 {
>         clk.rising_edge().await;
>         // The edge returns before the flop updates; read after it settles.
>         read_only().await;
>         assert_eq!(count.get_u64()?, i, "cycle {i}");
>         next_time_step().await;
>     }
>     Ok(())
> }
> ```
>
> (`reset` is a helper a few lines up in the same generated file.)

## Motivation

Verifying hardware means choosing between three unhappy options:

- **Write it in SystemVerilog.** Full access to the simulator, in a language
  with no package manager and no test runner, where randomisation and coverage
  work only on the simulators you pay for.
- **Write a Verilator C++ harness.** Fast, and now you hand-manage `eval()`
  calls, raw pointers into the model, and a rebuild every time the RTL moves.
  There is no timing model; you invent one.
- **Use cocotb.** The nicest of the three by a distance — a real scheduler,
  real triggers, a real language. But every trigger crosses into CPython, every
  value becomes a string on the way in and out, and `dut.conut` is a runtime
  `AttributeError` at 3 a.m. rather than a compile error.

cocotb's design is right and Rivet keeps it. What Rivet replaces is the
interpreter underneath it. The testbench is compiled, so the simulator talks to
native code across the same PLI it was already using, values move as
`aval`/`bval` word pairs instead of decimal strings, and the names of your
ports are checked before the simulation starts.

The cost is a compile step cocotb does not have. Measured rather than guessed,
it is **0.4 s on Icarus and 0.6 s on Verilator** for a one-line test edit, which
is the honest version of the tradeoff — and the reason Rivet does not bother
hot-loading the test library to shave it.

## Speed

Per simulated clock cycle, Icarus Verilog 12, 100k cycles:

| What the testbench does | cocotb 2.1 | Rivet |
|---|---|---|
| await an edge every cycle | 26.6 µs | **2.50 µs** |
| edge, write 32-bit, read 32-bit and 512-bit | 305 µs | **7.68 µs** |
| 100 tasks awaiting every edge | 311 µs | **20.8 µs** |

Micro-benchmarks flatter everyone, so there is also a real one: PicoRV32
running a program for 100k cycles costs 15.27 µs/cycle with no harness at all
and 19.99 µs/cycle driven by Rivet. The harness is about a quarter of the run,
and a monitor task watching the memory bus every cycle adds nothing measurable.
Method, medians, spread and caveats in [`docs/benchmarks.md`](docs/benchmarks.md).

## Features

- Works on four simulators, two HDLs, two PLIs — Icarus, Verilator, GHDL, NVC
- Seeded random stimulus and `#[derive(Randomize)]` with constraints; every run
  prints its seed and `--seed` replays it exactly
- Functional coverage — covergroups, bins, crosses — merged across parallel runs
- A kit: AXI4, AXI4-Lite, AXI4-Stream, APB, Avalon-MM, Wishbone, a sparse
  memory with `$readmemh`, scoreboards, checkers, golden traces
- `rivet bindgen` turns your hierarchy — `typedef enum`, `typedef struct packed`
  and all — into Rust types, so port names and widths are checked by rustc
- Hangs name themselves: a timeout prints every live task and what it awaits
- `rivet run -j 8` shards a regression across simulator processes
- `cargo test` and `cargo nextest` work, and so does cocotb-compatible
  `results.xml`
- It is an ordinary Rust crate: your editor, your debugger, `cargo add`
  anything you want in a testbench

## Simulators

| Simulator | Interface | Status |
|---|---|---|
| Icarus Verilog 12 | VPI | verified in CI, Linux and macOS |
| Verilator 5.020 / 5.036 | direct model access | verified in CI |
| GHDL 4.1 | VPI | verified in CI |
| NVC 1.17 | VHPI | verified in CI |
| Questa, Xcelium, VCS, Riviera, DSim | VPI / VHPI | code written, **never run** |

That last row is the one to read carefully. Rivet carries the launch flow and
every catalogued workaround for those five tools, but nobody has ever run it on
one. If you have a licence, [`examples/conformance`](examples/conformance) is
the suite to point at it, and
[`docs/SIMULATOR-QUIRKS.md`](docs/SIMULATOR-QUIRKS.md) marks every per-simulator
behaviour as observed or merely inherited from cocotb.

## Requirements

- [Rust](https://rustup.rs) 1.87 or later (checked in CI)
- One of: [Icarus Verilog](https://steveicarus.github.io/iverilog/) 11+,
  [Verilator](https://verilator.org) 5.x with a C++17 compiler,
  [GHDL](https://ghdl.github.io/ghdl/) 4.x, or [NVC](https://www.nickg.me.uk/nvc/) 1.17+
- macOS and Linux. Windows is not supported.

## Install

Rivet is not on crates.io yet, so build the CLI from a checkout:

```sh
git clone https://github.com/A-Ravioli/rivet && cd rivet
cargo build -p rivet-cli
```

Then scaffold a testbench crate that passes as generated:

```sh
target/debug/rivet new counter --path "$PWD"
target/debug/rivet run --sim icarus -C counter
```

```
TEST                           STATUS          SIM TIME        WALL
counter::counts_when_enabled   PASS               225ns      0.000s
counter::load_overrides_count  PASS                40ns      0.000s

RIVET_RESULT passed=2 failed=0 skipped=0 seed=2582011094885115739
rivet: 2 tests, 0 failed, 0 skipped (counter/sim_build/icarus/results.xml)
```

`--path` points the new crate at your checkout. Swap `--sim icarus` for
`verilator`, `ghdl` or `nvc`; the testbench does not change.

## How it works

Your test crate builds as a `cdylib`. `rivet run` compiles the design described
by `rivet.toml`, hands the simulator that library as a VPI or VHPI plugin, and
the simulator loads it the way it would load any other PLI module. Inside,
Rivet's executor runs on the simulator's own thread — no threads to
synchronise, no interpreter, no IPC. Triggers are simulator callbacks;
`.await` returns when the callback fires.

Verilator's VPI is too limited to drive a harness through, so there the crate
builds as a binary that links the Verilated model instead, and Rivet reads and
writes signals directly in the model's storage — using VPI only to be told
when a value changed.

The two chapters worth reading are
[the timing model](docs/book/src/timing-model.md) — what `ReadWrite` and
`ReadOnly` mean, and why an edge returns before the HDL reacts to it — and
[simulators](docs/book/src/simulators.md), on what each tool can and cannot
tell a harness.

## Related

- [cocotb](https://www.cocotb.org) — the project Rivet is a reimplementation of,
  and still the right answer if you need pyuvm, numpy or a commercial simulator
  today. [`docs/design/00-cocotb-analysis.md`](docs/design/00-cocotb-analysis.md)
  is a file-by-file study of it; every design decision here came out of that.
- [marlin](https://github.com/ethanuppal/marlin) — imports hardware modules
  into Rust as structs via procedural macros and `dlopen`, Verilator-focused,
  with support for Spade and Veryl. Different bet: Rivet keeps cocotb's
  simulator-agnostic timing model and drives four simulators through their
  standard PLIs.
- [verilated-rs](https://github.com/djg/verilated-rs) — statically links
  Verilated bindings from a build script; unmaintained.

## Development

```sh
# No simulator needed: the harness is tested against a pure-Rust one, and the
# CLI's end-to-end tests skip themselves when their tool is missing.
cargo test -p rivet-core -p rivet-mock -p rivet-kit -p rivet-cli
cargo test --workspace     # adds the examples, which do need Icarus
cargo clippy --workspace --all-targets -- -D warnings
mdbook serve docs/book
```

[`docs/testing.md`](docs/testing.md) says what each layer covers, what it does
not, and every defect the suites have caught so far.
[`docs/design/`](docs/design/README.md) has the architecture and the status of
each milestone.

## License

MIT or Apache-2.0, at your option.
