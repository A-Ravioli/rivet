# Implementation status

Where the code stands against [`02-roadmap.md`](02-roadmap.md).

| Milestone | Status | Notes |
|---|---|---|
| M0 executor + mock simulator | done | `rivet-core`, `rivet-mock`; 13 timing-model/executor tests plus value/time unit tests, no simulator needed |
| M1 Verilator | done | `rivet-verilator`: build helper, generated shim, Rust main loop, native timer wheel and phase scheduling, direct signal access; VCD/FST tracing; verified on 5.020 and 5.036 |
| M2 Icarus / general VPI | done | `rivet-vpi`: `vpiVectorVal` values, persistent value-change callbacks, callback re-entrancy handled in the runtime, `vlog_startup_routines` export; other VPI simulators carry cocotb's quirks in code but are unverified |
| M3 `cargo test` harness and CLI | done | `rivet` CLI (`run`, `build`, `bindgen`, `clean`, `--filter`, `--waves`, `--release`, `--seed`, `--log`), `rivet.toml`, `#[rivet::test]` with `timeout`/`skip`/`expect_fail`/`stage`, JUnit `results.xml`; `rivet::harness::main()` makes `cargo test -p <crate>` run the simulation (`RIVET_SIM` selects the simulator) with `--list`, filters, `--exact`, `--skip`, and libtest-style output. `cargo nextest` is untested |
| M4 VHDL | partial | VHDL designs run on GHDL through its VPI (`examples/dff_vhdl`, values as binary strings since GHDL has no `vpiVectorVal`); generics are not reachable by name on GHDL; a VHPI backend (NVC, Riviera, Xcelium VHDL, Questa) is not started |
| M5 typed bindings, Verilator direct access | partial | `rivet bindgen --sim <sim>` runs the design once, dumps the hierarchy as JSON, and generates a typed module (`examples/dff/src/dut.rs`); tests take `dut: Dut` via the `Bind` trait; widths are checked at bind time; constants inside generate blocks are left out so bindings stay portable across simulators. Verilator direct signal access is done: values go straight to the model's storage through `VerilatedScope::varFind`, VPI is used only for value-change callbacks |
| M6 kit | partial | `Clock`, `Event`, `Queue`, `Lock`, `first`/`join`/`with_timeout`, `Scope`; `rivet-kit` has `reset`, `Driver`/`Monitor`, a valid/ready source and sink, and an in-order `Scoreboard` (`examples/fifo`); no AXI or memory models yet |
| M7 commercial simulators | code only | Xcelium startup, Questa string-write, Verilator recurring-callback quirks are implemented but have never run on those tools |

## Verified end to end

- `examples/dff`: 10 tests on Icarus Verilog 12.0 and Verilator 5.020, covering
  generated typed bindings, generate blocks,
  edges, ReadWrite/ReadOnly, deposits, X before reset, parameters, hierarchy
  enumeration, unpacked arrays, `integer`, `real`, timeouts, `expect_fail`,
  concurrent tasks with a `Queue`.
- `examples/bench`: 7 benchmarks on both; numbers in
  [`../benchmarks.md`](../benchmarks.md).
- `examples/fifo`: 3 kit-based tests (valid/ready source from a queue, sink
  with random backpressure, scoreboard) on Icarus and Verilator, with
  identical simulated end times on both.
- `examples/dff_vhdl`: 2 tests on GHDL 4.1.
- `examples/conformance`: 21 tests per simulator on Icarus, Verilator 5.020,
  and Verilator 5.036, covering every backend feature; see
  [`../testing.md`](../testing.md) for the coverage map and the defects the
  suite found.

## Behaviours pinned by tests against the mock

- Edge triggers return in the values-change phase, before downstream HDL.
- Deposits are not readable back until the simulator evaluates; buffered
  deposits are flushed at the start of ReadWrite; latest write wins.
- Writing or awaiting ReadWrite/ReadOnly in the ReadOnly phase panics.
- Cancelling a task drops its future and deregisters its trigger.
- A panic in any task fails the current test.
- FIFO task scheduling.

The plan for everything below and for the unverified rows above is in
[`04-remaining-work.md`](04-remaining-work.md).

## Known gaps and decisions

- Deposits on Verilator (every version) are buffered by the runtime until
  ReadWrite and applied as immediate writes; Rivet does not use Verilator's
  `vpiInertialDelay` machinery, which would need `doInertialPuts` calls in
  the loop. Verified identical behaviour on 5.020 and 5.036.
- Force/Release are reported unsupported on Verilator (`Capabilities`).
- `Signal::index` and `member` work for unpacked arrays and struct members
  the simulator exposes through `vpi_handle_by_index`/`by_name`; generate
  arrays use cocotb's pseudo-region fallback (`dut.path_signal("gen[1].tap")`,
  `dut.module("gen")?.index(1)`), tested on Icarus and Verilator.
- The test crate must be linked into the Verilator binary
  (`use example_dff as _;` in `main.rs`) for `inventory` registrations to be
  present.
