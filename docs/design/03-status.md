# Implementation status

Where the code stands against [`02-roadmap.md`](02-roadmap.md).

| Milestone | Status | Notes |
|---|---|---|
| M0 executor + mock simulator | done | `rivet-core`, `rivet-mock`; timing-model, executor, runner and kit suites run without a simulator |
| M1 Verilator | done | `rivet-verilator`: build helper (per-parameter-set object directories), generated shim, Rust main loop, native timer wheel and phase scheduling, direct signal access; VCD/FST tracing with per-test files; verified on 5.020 and 5.036 |
| M2 Icarus / general VPI | done | `rivet-vpi`: `vpiVectorVal` values, persistent value-change callbacks, callback re-entrancy handled in the runtime, `vlog_startup_routines` export, `$dumpon`/`$dumpoff` control; other VPI simulators carry cocotb's quirks in code but are unverified |
| M3 `cargo test` harness and CLI | done | `rivet run|build|watch|bindgen|cov|clean`; `-j N` shards, `--param-set`, `--seed`, `--wall-timeout`, `--waves[-per-test]`, `--log-format json`, per-test log files, `--cov-threshold`, `--update-golden`, `--manifest`; `cargo-rivet`; `#[rivet::test]` with `timeout`/`wall_timeout`/`skip`/`expect_fail`/`stage`/`params`/`param_sets`; JUnit `results.xml` plus `results.json`; `rivet::harness::main()` for `cargo test` with `--test-threads` |
| M4 VHDL | partial | VHDL designs run on GHDL through its VPI (`examples/dff_vhdl`); generics are not reachable by name on GHDL; no VHPI backend yet |
| M5 typed bindings, Verilator direct access | done | `rivet bindgen` generates a module per instance, `Vec<Signal>` for arrays, Rust enums and bit-layout structs for `typedef enum` / `typedef struct packed` found in the sources, and `Dut::hierarchy()`; Verilator direct access through `VerilatedScope::varFind` |
| M6 kit | done | `Clock` (phase, jitter), `Reset` builder, `Event`/`Queue`/`Lock`, `Scope`; `rivet-kit`: `Driver`/`Monitor`, valid/ready, `Scoreboard`, `Model`/`ModelScoreboard`, `Memory` with `$readmemh`, AXI4-Lite/AXI4/AXI4-Stream/APB/Avalon-MM/Wishbone masters and memory-backed slaves with backpressure, checkers (`assert_*`, X detection), golden `Trace`s |
| M7 commercial simulators | code only | Xcelium startup, Questa string-write, Verilator recurring-callback quirks are implemented but have never run on those tools |

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

## Verified end to end

- `examples/dff`: 10 tests on Icarus Verilog 12.0 and Verilator 5.020/5.036.
- `examples/bench`: 7 benchmarks on both; numbers in
  [`../benchmarks.md`](../benchmarks.md).
- `examples/fifo`: 3 kit-based tests on Icarus and Verilator.
- `examples/dff_vhdl`: 2 tests on GHDL 4.1.
- `examples/conformance`: 21 tests per simulator on Icarus, Verilator 5.020
  and 5.036.
- `examples/bus`: 13 tests, run under two parameter sets (25 results) on
  Icarus, Verilator 5.020 and 5.036, serially and sharded, with golden
  traces shared between simulators.
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

The plan for everything still open is in
[`04-remaining-work.md`](04-remaining-work.md).

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
