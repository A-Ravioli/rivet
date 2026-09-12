# Testing

What is tested, where, and what the suites found.

## Layers

| Layer | Where | Needs | Count |
|---|---|---|---|
| Values (`LogicVec`, `Logic`), time units and rounding, Fx hash, result selection and XML escaping, seeded random (`Rng`, ranges, weights, per-test streams), functional coverage (bins, ignore/illegal, crosses, JSON), JSON log records and per-test log files | `crates/rivet-core/src/*.rs` unit tests | nothing | 36 |
| Manifest parsing, parameter sets | `crates/rivet-manifest` | nothing | 2 |
| `#[rivet::test]` attribute parsing (`timeout`, `wall_timeout`, `params`, `param_sets`) | `crates/rivet-macros` | nothing | 1 |
| Bindgen code generation (parsed back with `syn`, typedef enums and packed structs, Icarus-internal scopes skipped), SystemVerilog typedef parsing and literal evaluation, coverage merging and rendering, watch fingerprints, results.json round trip, sharding, libtest-style selection, argument parsing | `crates/rivet-cli/src/*.rs` unit tests | nothing | 14 |
| Timing model and executor semantics: edge, ReadWrite, ReadOnly, NextTimeStep phases; deposit buffering and dedup; trusted-inertial mode; force/release; timers with equal deadlines; drop-deregistration; multiple waiters per edge; scopes; join handles; child panics; `yield`/`join`/`with_timeout`; `Queue`, `Lock`, `Event`; clock builder options; signal and hierarchy API; constant writes; determinism | `crates/rivet-mock/tests/{core,semantics}.rs` | nothing (mock simulator) | 35 |
| The regression runner: ordering, skip, `expect_fail`, timeouts, invalid timeouts, panics, filters, `results.xml` round trip, hierarchy dump, HDL-side finish, parameter-set selection | `crates/rivet-mock/tests/regression.rs` | nothing | 6 |
| `#[derive(Randomize)]`: ranges, `one_of`, weighted, `with`, `skip`, enum weights and skipped variants, constraints, unsatisfiable constraints, generics; per-test seeds through the runner, stable under filtering, recorded in `results.xml` | `crates/rivet-mock/tests/random.rs` | nothing | 4 |
| Hang diagnostics: the task dump names every task and its trigger; simulated-time timeouts include it; the wall-clock limit fails a test in-band; the watchdog aborts a stuck process (child process) | `crates/rivet-mock/tests/diagnostics.rs` | nothing | 3 |
| JSON logs and per-test log files through the runner | `crates/rivet-mock/tests/logging.rs` | nothing | 1 |
| Clock phase offsets and reproducible jitter; the `Reset` builder (sync and async); waveform commands and per-test wave files | `crates/rivet-mock/tests/clocks_waves.rs` | nothing | 5 |
| Bus models in loopback (kit master against kit slave with random wait states): AXI4-Lite with monitor and error ranges, AXI4 INCR/WRAP/FIXED bursts, narrow transfers, IDs and errors, AXI4-Stream packets with gaps and backpressure, APB, Avalon-MM simple and pipelined, Wishbone | `crates/rivet-mock/tests/bus.rs` | nothing | 6 |
| Checkers passing and failing the test (`assert_stable`, `assert_within`, `assert_becomes`, `assert_never`, `assert_always`, `assert_implies`, `assert_no_x`, `find_x`), model scoreboard, golden traces (missing, accepted, matching, differing) | `crates/rivet-mock/tests/kit_checks.rs` | nothing | 5 |
| Memory model words, strobes, pages, `$readmemh` parsing with X/Z, hex and binary loading, hex dumps; AXI burst addressing; trace diffs | `crates/rivet-kit` unit tests | nothing | 5 |
| CLI end to end: run, filter, exit codes, HDL `$finish` reported as failure, waveform output, `bindgen` output parses and matches the committed bindings, error paths, Verilator and GHDL flows, sharded runs with parameter sets and merged coverage, coverage thresholds, `cov report`, golden diffs, `watch`, `cargo-rivet`, JSON logs and per-test log files | `crates/rivet-cli/tests/cli.rs` | Icarus, Verilator, GHDL (each test skips if its tool is missing) | 9 |
| `cargo test` integration | `examples/dff/tests/sim.rs`, `examples/bus/tests/sim.rs` via `rivet::harness` (`--test-threads` shards) | Icarus | 10 + 25 |
| Conformance on real simulators: cocotb's inertial-write cases, phases, timer precision, combinational visibility, edges, X before reset, 128-bit vectors, signed values, packed structs, string/integer/real variables, memory arrays, parameters, generate instances, force/release, discovery, plusargs, mass cancellation, HDL `$finish` | `examples/conformance` | Icarus, Verilator 5.020 and 5.036 | 21 per simulator |
| Kit against real RTL: AXI4-Lite, APB, Wishbone and Avalon register files, an AXI4 memory with bursts, an AXI-Stream skid buffer, a register slice into the kit's AXI4-Lite slave, a typed ALU through generated `Cmd`/`Op` types; random stimulus with coverage, golden traces per parameter set, checkers, `params`, `param_sets`, waveform windows, wall-clock limits | `examples/bus` | Icarus, Verilator 5.020 and 5.036 | 13 tests, 25 per simulator over two parameter sets |
| Kit (valid/ready, scoreboard) | `examples/fifo` | Icarus, Verilator | 3 per simulator |
| VHDL | `examples/dff_vhdl` | GHDL | 2 |
| FuseSoC and Edalize: the `rivet` tool backend generates `rivet.toml` from a core file and runs the dff example | `integrations/edalize`, `examples/dff/rivet-dff.core` | Icarus, Python | 1 flow |
| `rivet_py`: memory, RNG (bit-identical to Rust), scoreboard and coverage from Python; a cocotb test using them | `python/rivet_py/tests` | Python, cocotb, Icarus | 4 + 1 |

Run everything with:

```sh
cargo test --workspace --no-fail-fast          # unit, mock, CLI e2e, cargo-test harness
for e in dff fifo conformance bus; do
  target/debug/rivet run --sim icarus    -C examples/$e
  target/debug/rivet run --sim verilator -C examples/$e
done
target/debug/rivet run --sim icarus -C examples/bus -j 4 --cov-threshold 99
target/debug/rivet run --sim ghdl -C examples/dff_vhdl
pip install -e integrations/edalize && fusesoc --cores-root examples/dff run --no-export --tool rivet rivet:examples:dff
(cd python/rivet_py && maturin build --release -o dist && pip install dist/*.whl && cd tests && python -m unittest test_shim && cd cocotb_dff && python run.py)
```

CI runs all of the above on Ubuntu 24.04 with apt Icarus 12, Verilator
5.020 and GHDL 4.1, a second job with Verilator 5.036 built from source,
and a third job for the Python integrations.

## What the suites found

The first expansion (unit, mock, conformance) surfaced six defects, all
fixed:

1. **A bad `timeout` expression aborted the whole regression.** Both the
   evaluation and the conversion to steps now run under the guard.
2. **Writing a constant was silently dropped.** The check now happens at
   the call site and fails the test.
3. **Signal paths disagreed between simulators.** Paths are built from
   parent path plus name; generate elements are `parent[i]`.
4. **Verilator direct access could write the wrong storage.** Root-level
   names resolve the `TOP` scope first.
5. **Verilator parameters were writable.** The symbol table's `isParam`
   marks them constant.
6. **`IntoLogicVec` was missing from the prelude.**

The second expansion (kit, coverage, sharding, bindgen, integrations)
found:

7. **Sharded and parameter-set runs each drew their own seed.** The CLI
   now chooses one base seed per run and passes it to every process.
8. **Traces stamped with absolute time could not be golden.** A test's
   start time depends on what ran before it and on sharding; stamps are
   now relative to the trace's creation, and goldens are per parameter set.
9. **Bindgen emitted Icarus-internal scopes** (`$ivl_for_loop0`,
   `$unm_blk_3`), which do not exist on Verilator. They are skipped.
10. **Verilator's `--Mdir` for a parameter set was not created**, and the
    tool-version stamp was shared between sets, so switching Verilator
    versions rebuilt one set and linked stale objects for the next. Each
    set now has its own directory and stamp.
11. **Empty coverage files were written for crates without covergroups**
    and reported as 100%. Nothing is written or reported without groups.

Test-writing again caught timing-model mistakes (writes in the ReadOnly
phase after a checker or a `read_only().await`), which is why the immediate
checkers now return at the start of the next time step.

## Simulator differences the suite documents

| Property | Icarus 12 | Verilator 5.020 / 5.036 | GHDL 4.1 |
|---|---|---|---|
| Deposit readable back in the same ReadWrite phase | no | yes (applied as an immediate write at the flush) | not tested |
| `string` variables visible through VPI | no | yes | n/a |
| Packed struct members addressable by name | no | no | n/a |
| Four-state values (X before reset) | yes | no (two-state) | yes |
| Force/release | yes | reported unsupported | not tested |
| Generics/parameters by name | yes | yes (marked constant from the symbol table) | no |
| Real and string parameters | yes | yes | n/a |
| Waveform control from the test | on/off (`$dumpon`/`$dumpoff`), one file per run | on/off and a new file per call | none (`--wave` covers the run) |
| Internal scopes in the hierarchy | `$ivl_*`, `$unm_blk_*` (skipped by bindgen) | none | n/a |

## Not covered

- Commercial simulators (no licence); the conformance crate is what a
  licence holder should run.
- VHPI (no backend yet); GHDL runs through VPI.
- Mixed-language designs.
- macOS and Windows.
- The Edalize backend is exercised through FuseSoC only; Edalize's flow
  API (`edalize.tools`) is not implemented.
