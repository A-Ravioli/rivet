# Implementation status

Where the code stands against [`02-roadmap.md`](02-roadmap.md).

| Milestone | Status | Notes |
|---|---|---|
| M0 executor + mock simulator | done | `rivet-core`, `rivet-mock`; timing-model, executor, runner and kit suites run without a simulator |
| M1 Verilator | done | `rivet-verilator`: build helper (per-parameter-set object directories), generated shim, Rust main loop, native timer wheel and phase scheduling, direct signal access; VCD/FST tracing with per-test files; verified on 5.020 and 5.036 |
| M2 Icarus / general VPI | done | `rivet-vpi`: `vpiVectorVal` values, persistent value-change callbacks, callback re-entrancy handled in the runtime, `vlog_startup_routines` export, `$dumpon`/`$dumpoff` control; other VPI simulators carry cocotb's quirks in code but are unverified |
| M3 `cargo test` harness and CLI | done | `rivet run|build|watch|bindgen|cov|clean`; `-j N` shards, `--param-set`, `--seed`, `--wall-timeout`, `--waves[-per-test]`, `--log-format json`, per-test log files, `--cov-threshold`, `--update-golden`, `--manifest`; `cargo-rivet`; `rivet new` scaffolds a crate that runs as generated; `--filter` takes regular expressions and `--shuffle` reorders within a stage; `#[rivet::test]` with `timeout`/`wall_timeout`/`skip`/`expect_fail`/`expect_fail = "message"`/`expect_timeout`/`stage`/`params`/`param_sets`, and `#[rivet::fixture]` for setup a test asks for by name; `rivet::skip(reason)` and `rivet::runtime::finish_test()` end a test at run time; JUnit `results.xml` (with `file` and `line`) plus `results.json`; `rivet::harness::main()` for `cargo test` with `--test-threads`, and `cargo nextest run` with one test per process |
| M4 VHDL | done on NVC, partial on GHDL | `rivet-vhpi` implements the `Backend` trait over VHPI and `rivet run --sim nvc` analyses, elaborates and runs; verified on NVC 1.23. NVC exposes record members, enumeration literal names, integers, booleans, generics by name and each for-generate element as its own region. GHDL still runs through its VPI, where records, enumeration literals and generics are not reachable, so the tests that need them call `rivet::skip`. Missing on both: waveform control from a test (the simulator's `--wave` covers the run), force/release exercised on hardware, and FLI |
| M5 typed bindings, Verilator direct access | done | `rivet bindgen` generates a module per instance, `Vec<Signal>` for arrays and `Vec<Vec<Signal>>` for two-dimensional ones, Rust enums and bit-layout structs for `typedef enum` / `typedef struct packed` found in the sources, and `Dut::hierarchy()`; output is formatted, so regenerating committed bindings produces no diff. `Signal::slice(hi, lo)` reads and writes a bit range on every backend; `Signal::enum_literals()`/`enum_name()` answer where the simulator reports them (VHPI). Verilator direct access through `VerilatedScope::varFind` |
| M6 kit | done | `Clock` (phase, jitter), `Reset` builder, `Event`/`Queue`/`Lock`, `Scope`; `rivet-kit`: `Driver`/`Monitor`, valid/ready, `Scoreboard`, `Model`/`ModelScoreboard`, `Memory` with `$readmemh`, AXI4-Lite/AXI4/AXI4-Stream/APB/Avalon-MM/Wishbone masters and memory-backed slaves with backpressure, checkers (`assert_*`, X detection), golden `Trace`s |
| M7 commercial simulators | code only | `rivet run --sim questa|xcelium|vcs|riviera|dsim` builds a command line, compiles the design and loads the harness, and the backends carry the per-simulator quirks those tools need (Xcelium startup, Questa string writes, recurring callbacks). None of it has ever run on the tools; [`../SIMULATOR-QUIRKS.md`](../SIMULATOR-QUIRKS.md) says which workaround is verified and which is carried from cocotb's catalogue |

Beyond the roadmap:

- Seeded random stimulus: `rivet::rng()` per-test streams derived from the
  run seed and the test name, `#[derive(Randomize)]` with ranges, weights
  and rejection-sampled constraints; the seed is printed by every run and
  recorded per test in `results.xml`.
- Functional coverage: covergroups, points with named/auto/split bins,
  ignore and illegal ranges, crosses; `coverage.json` per run merged across
  shards and parameter sets; `rivet cov report` and thresholds.
- Hang diagnostics: every task's pending trigger is known; timeouts print
  the task list; per-test wall-clock limits with an in-band failure and a
  watchdog abort.
- Integrations: an Edalize tool backend (`fusesoc run --tool rivet`) and
  `rivet_py`, the simulator-independent kit for cocotb testbenches.
  [`../migration.md`](../migration.md) maps cocotb onto Rivet.
- Release plumbing: crates.io metadata on every publishable crate,
  `CHANGELOG.md`, `.github/workflows/release.yml` (cross-built binaries, the
  `rivet_py` wheel, publishing in dependency order),
  `ci/check-package-metadata.py`, and a CI job pinning the 1.87 MSRV.
- The user-facing book in [`../book`](../book), built by CI.

## Verified end to end

- `examples/dff`: 11 tests on Icarus Verilog 12.0 and Verilator 5.020/5.036,
  including a `#[rivet::fixture]` that starts the clock and releases reset.
- `examples/bench`: 7 benchmarks on both; numbers in
  [`../benchmarks.md`](../benchmarks.md).
- `examples/fifo`: 3 kit-based tests on Icarus and Verilator.
- `examples/dff_vhdl`: 2 tests on GHDL 4.1 and on NVC 1.23.
- `examples/vhdl_types`: 6 tests covering a record port, enumeration literal
  names, a boolean, an integer, an unconstrained generic and a for-generate
  region. On NVC 5 pass and 1 skips (an opt-in hierarchy dump); on GHDL 2
  pass and 4 skip, because its VPI exposes neither record members nor
  enumeration literals.
- `examples/conformance`: 21 tests per simulator on Icarus, Verilator 5.020
  and 5.036.
- `examples/bus`: 13 tests, run under two parameter sets (25 results) on
  Icarus, Verilator 5.020 and 5.036, serially and sharded, with golden
  traces shared between simulators.
- `cargo nextest run` on `examples/dff` (11 tests) and `examples/bus` (13),
  each test in its own process on Icarus.
- `rivet new demo` scaffolds a crate whose 2 tests pass as generated on
  Icarus and Verilator, with no edits.
- FuseSoC/Edalize and `rivet_py` flows on Icarus.

See [`../testing.md`](../testing.md) for the coverage map and the defects
the suites found.

## Behaviours pinned by tests against the mock

- Edge triggers return in the values-change phase, before downstream HDL.
- Deposits are not readable back until the simulator evaluates; buffered
  deposits are flushed at the start of ReadWrite; latest write wins.
- Writing or awaiting ReadWrite/ReadOnly in the ReadOnly phase panics.
- Cancelling a task drops its future and deregisters its trigger.
- A panic in any task fails the current test.
- FIFO task scheduling.
- The same seed reproduces the same stimulus, clock jitter and bus wait
  states, whatever ran before the test and however the run is sharded.
- `--shuffle` reorders tests within a stage only, and the order replays
  from the run's seed.
- Filters are regular expressions searched against `module::name` and the
  bare name, with a literal fallback so a parametrised name such as
  `axi_mem_bursts[16]` still selects itself.
- `rivet::skip(reason)` ends the running test as skipped and
  `rivet::runtime::finish_test()` ends it as passed, from any task.
- `expect_fail = "message"` fails a test that fails for a different reason;
  `expect_timeout` expects simulated time to run out.
- A fixture's setup runs once per test that asks for it, and its teardown
  runs when the value drops, however the test ended.
- A slice write starts from a write already buffered in the same time step,
  so successive slice writes compose.

The plan for everything still open is in
[`04-remaining-work.md`](04-remaining-work.md); the phased plan for the
places cocotb is still ahead is in [`05-parity-plan.md`](05-parity-plan.md).

## Known gaps and decisions

- Deposits on Verilator (every version) are buffered by the runtime until
  ReadWrite and applied as immediate writes; Rivet does not use Verilator's
  `vpiInertialDelay` machinery. Verified identical behaviour on 5.020 and
  5.036.
- Force/Release are reported unsupported on Verilator (`Capabilities`).
- Generate arrays use cocotb's pseudo-region fallback
  (`dut.path_signal("gen[1].tap")`), tested on Icarus and Verilator.
- Sharded runs (`-j`) assume tests are independent: `stage` ordering does
  not hold across shards, and a test that leaves the design in a state the
  next test relies on must not be sharded.
- Golden traces are per parameter set and use time stamps relative to the
  trace's creation; a design whose timing differs between simulators needs
  per-simulator goldens (not supported; use `Trace::unstamped`).
- The test crate must be linked into the Verilator binary
  (`use example_dff as _;` in `main.rs`) for `inventory` registrations to be
  present.
