# Rivet roadmap

Milestones are ordered so that each one produces something runnable and
testable, and so that the free simulators (Verilator, Icarus, GHDL, NVC) are
covered before any commercial one. Each milestone lists its exit criteria.

## Crate layout

```
rivet/
  Cargo.toml                 workspace
  crates/
    rivet-core/              executor, triggers, tasks, scope, timer wheel, values, handles, Backend trait
    rivet-mock/              pure-Rust event simulator implementing Backend
    rivet-vpi/               VPI backend (bindgen over vendored vpi_user.h), per-sim feature flags
    rivet-vhpi/              VHPI backend
    rivet-fli/               FLI backend (Questa)
    rivet-verilator/         Verilator backend: main loop, C++ shim, build.rs support
    rivet-bindgen/           hierarchy dump -> typed Rust bindings (generic + Verilator XML)
    rivet-macros/            #[rivet::test], rivet::main!, params
    rivet-kit/               Clock, Reset, Event, Queue, Lock, combinators, Driver/Monitor/Scoreboard, bus models
    rivet-runner/            simulator build/run orchestration, JUnit/JSON reporting, libtest-compatible harness
    rivet-cli/               `rivet` binary (build, run, list, dump-hierarchy, bench)
    rivet/                   facade crate re-exporting the public API
  examples/                  ports of cocotb's examples (adder, dff, matrix_multiplier, mixed_language, ...)
  benches/                   edge round trip, value traffic, task scaling, startup; cocotb baselines
  docs/design/               these documents
```

## M0: executor and mock simulator

Goal: the concurrency model and timing model exist and are tested without any
EDA tool.

- `rivet-core`: `Task`, `spawn`, `Scope`, FIFO ready queue, `run_until_idle`,
  deterministic wakers, cancellation by drop.
- Triggers as futures: `Timer`, `NextTimeStep`, `ReadWrite`, `ReadOnly`,
  `ValueChange`, `RisingEdge`, `FallingEdge`, `Event`, `JoinHandle`.
- Phase tracking and the illegal-transition errors from cocotb's timing model.
- Pending-write buffer with both `trusts_inertial_writes` modes.
- `LogicVec`, `Logic`, `Bits<N>`, conversions, literals, display.
- `rivet-mock`: a delta-cycle event simulator with processes written in Rust
  closures, timestep phases, inertial and no-delay writes, value-change
  events. Enough to model a clocked register and a combinational adder.
- Tests: every example in cocotb's `docs/source/timing_model.rst` as a mock
  test; the `test_inertial_writes` cases; cancellation and scope semantics.

Exit: `cargo test` in the workspace passes with no simulator installed.

## M1: Verilator

Goal: a real design runs end to end at native speed.

- `rivet-verilator`: `build.rs` helper that invokes Verilator, generated
  `extern "C"` shim, `main` loop reproducing cocotb's `verilator.cpp`
  semantics (eval-step loop, `ReadWrite`, `ReadOnly`, next-time selection,
  timed callbacks, `NextSimTime`, trace and coverage).
- `rivet-vpi` minimum: value read/write via `vpiVectorVal`, hierarchy
  discovery, the five callback kinds, using Verilator's VPI. Verilator's
  quirks: recurring callbacks removed after fire, `fatalOnVpiError(false)`.
- Native `Clock` on the timer wheel.
- `#[rivet::test]` and a minimal sequential test runner in-process.
- Examples: `dff`, `adder` ported from cocotb.

Exit: `cargo run -p example-dff` runs the cocotb dff tests on Verilator with
identical pass/fail behaviour; the edge round-trip benchmark runs.

## M2: Icarus (the general VPI path)

Goal: the plugin topology works with an external event-driven simulator.

- `cdylib` build with `vlog_startup_routines` export; `vvp -m` loading.
- Callback re-entrancy queue (cocotb `gh-4067`).
- Untrusted inertial writes mode on by default for Icarus.
- `rivet-runner`: compile (`iverilog`) and run (`vvp`) with parameters,
  defines, includes, plusargs, FST waves; content-hashed build cache.
- Results back to the parent over a pipe; JUnit `results.xml`.

Exit: the same examples pass on Icarus and Verilator; the benchmark suite has
cocotb 2.2 baselines for both.

## M3: `cargo test` harness and CLI

- `rivet::main!` libtest-compatible harness: `--list`, filters, `--format
  json`, exit codes, `--test-threads` mapping to simulator processes.
- `rivet` CLI: `build`, `run`, `list`, `dump-hierarchy`, `bench`.
- `rivet.toml` / `[package.metadata.rivet]` schema.
- Test attributes: `timeout`, `expect_fail`, `skip`, `stage`, parameter
  matrices.
- `tracing` subscriber with sim-time prefix and per-test capture.
- Panic handling at every FFI entry; test isolation between tests in one
  process.

Exit: `cargo test` and `cargo nextest run` work on a Rivet test crate for
both simulators; CI runs the examples on Linux and macOS.

## M4: VHDL via VHPI (GHDL, NVC)

- `rivet-vhpi`: bindgen over `vhpi_user.h`; type classification for records,
  enums, arrays of arrays, `std_logic` vs `bit`, integer/real/string; name
  canonicalization; value transport via `vhpiLogicVecVal` with element
  lookup tables.
- Runner support for `ghdl` and `nvc`.
- Mixed-language discovery through both backends in one process.

Exit: cocotb's VHDL examples pass on GHDL and NVC.

## M5: typed bindings

- `rivet-bindgen` from Verilator XML and from a `dump-hierarchy` JSON.
- `Dut::bind(root)` with startup validation.
- Verilator direct-access shim for values.

Exit: the `matrix_multiplier` example has a typed `Dut`; the value-traffic
benchmark on Verilator direct shows no VPI calls on the hot path.

## M6: kit

- `Driver`/`Monitor`/`Scoreboard`, channels, valid/ready and AXI-Lite models,
  memory model, `Reset`.
- Migration guide: a cocotb-to-Rivet table covering every public cocotb
  symbol.

## M7: commercial simulators

Ported from cocotb's runner and GPI with each quirk cited, marked unverified
until run on real licenses:

- Questa/ModelSim: VPI, VHPI, and FLI (`rivet-fli`); `-pli`, `-foreign`,
  string-variable `vpiNoDelay` quirk, callback queue.
- Xcelium: `-loadvpi`/`-loadvhpi`, `cbAfterDelay(0)` startup, handle release
  after fire, callback queue.
- VCS: PLI table, handle release after fire.
- Riviera-PRO, Active-HDL, DSim, CVC.

## Ongoing

- Benchmarks in CI with regression thresholds.
- A `SIMULATOR-QUIRKS.md` that lists every workaround, its source in cocotb,
  and whether Rivet has verified it.
