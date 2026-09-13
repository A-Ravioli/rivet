# Testing

What is tested, where, and what the suites found.

## Layers

| Layer | Where | Needs | Count |
|---|---|---|---|
| Values (`LogicVec`, `Logic`), time units and rounding, Fx hash, result selection and XML escaping, seeded random (`Rng`, ranges, weights, per-test streams), functional coverage (bins, ignore/illegal, crosses, JSON), JSON log records and per-test log files | `crates/rivet-core/src/*.rs` unit tests | nothing | 36 |
| Manifest parsing, parameter sets | `crates/rivet-manifest` | nothing | 2 |
| `#[rivet::test]` attribute parsing (`timeout`, `wall_timeout`, `params`, `param_sets`) | `crates/rivet-macros` | nothing | 1 |
| Bindgen code generation (parsed back with `syn`, typedef enums and packed structs, two-dimensional array binders, Icarus-internal scopes skipped), SystemVerilog typedef parsing and literal evaluation, coverage merging and rendering, watch fingerprints, results.json round trip, sharding, libtest-style selection, argument parsing, the `rivet new` scaffold | `crates/rivet-cli/src/*.rs` unit tests | nothing | 16 |
| VHPI decoding: quoted literals, an enumeration classified by its literals (so `std_logic`, `bit`, `boolean` and `character` are told apart), VHPI time round trip | `crates/rivet-vhpi/src/lib.rs` unit tests | nothing | 3 |
| Timing model and executor semantics: edge, ReadWrite, ReadOnly, NextTimeStep phases; deposit buffering and dedup; trusted-inertial mode; force/release; timers with equal deadlines; drop-deregistration; multiple waiters per edge; scopes; join handles; child panics; `yield`/`join`/`with_timeout`; `Queue`, `Lock`, `Event`; clock builder options; signal and hierarchy API; constant writes; determinism | `crates/rivet-mock/tests/{core,semantics}.rs` | nothing (mock simulator) | 35 |
| `Signal::slice(hi, lo)`: reading and writing a bit range through the whole vector, and successive slice writes in one time step composing instead of the later one dropping the earlier | `crates/rivet-mock/tests/core.rs` | nothing | 1 |
| The regression runner: ordering, skip, `expect_fail`, timeouts, invalid timeouts, panics, filters, `results.xml` round trip, hierarchy dump, HDL-side finish, parameter-set selection | `crates/rivet-mock/tests/regression.rs` | nothing | 6 |
| Runner parity with cocotb: regular-expression filters, `--shuffle` within a stage replayed from the run seed, `expect_fail = "message"`, `expect_timeout`, `rivet::runtime::finish_test()`, file and lineno in `results.xml` | `crates/rivet-mock/tests/regression.rs` | nothing | 4 |
| `#[rivet::fixture]`: async setup injected into a test by argument name, torn down when the value drops | `crates/rivet-mock/tests/regression.rs` (`fixture_runs_setup_and_drops_afterwards`), `examples/dff` (`fixture_gives_a_running_clock`) | nothing / Icarus and Verilator | 1 + 1 |
| `#[derive(Randomize)]`: ranges, `one_of`, weighted, `with`, `skip`, enum weights and skipped variants, constraints, unsatisfiable constraints, generics; per-test seeds through the runner, stable under filtering, recorded in `results.xml` | `crates/rivet-mock/tests/random.rs` | nothing | 4 |
| Hang diagnostics: the task dump names every task and its trigger; simulated-time timeouts include it; the wall-clock limit fails a test in-band; the watchdog aborts a stuck process (child process) | `crates/rivet-mock/tests/diagnostics.rs` | nothing | 3 |
| JSON logs and per-test log files through the runner | `crates/rivet-mock/tests/logging.rs` | nothing | 1 |
| Mixed-language routing (`CompositeBackend`): handle and callback-id tagging, a lookup crossing the language boundary by fully qualified path, reads, writes and value-change callbacks routed to the owning half while time stays on the primary, and refusal when the halves disagree about precision or a secondary cannot tag its events | `crates/rivet-mock/tests/composite.rs` | nothing (two composed mock backends) | 8 |
| Clock phase offsets and reproducible jitter; the `Reset` builder (sync and async); waveform commands and per-test wave files | `crates/rivet-mock/tests/clocks_waves.rs` | nothing | 5 |
| Bus models in loopback (kit master against kit slave with random wait states): AXI4-Lite with monitor and error ranges, AXI4 INCR/WRAP/FIXED bursts, narrow transfers, IDs and errors, AXI4-Stream packets with gaps and backpressure, APB, Avalon-MM simple and pipelined, Wishbone | `crates/rivet-mock/tests/bus.rs` | nothing | 6 |
| Checkers passing and failing the test (`assert_stable`, `assert_within`, `assert_becomes`, `assert_never`, `assert_always`, `assert_implies`, `assert_no_x`, `find_x`), model scoreboard, golden traces (missing, accepted, matching, differing) | `crates/rivet-mock/tests/kit_checks.rs` | nothing | 5 |
| Memory model words, strobes, pages, `$readmemh` parsing with X/Z, hex and binary loading, hex dumps; AXI burst addressing; trace diffs | `crates/rivet-kit` unit tests | nothing | 5 |
| CLI end to end: run, filter, exit codes, HDL `$finish` reported as failure, waveform output, `bindgen` output parses and matches the committed bindings, error paths, Verilator, GHDL and NVC flows, sharded runs with parameter sets and merged coverage, coverage thresholds, `cov report`, golden diffs, `watch`, `cargo-rivet`, JSON logs and per-test log files, `rivet new` scaffolding a crate and running it | `crates/rivet-cli/tests/cli.rs` | Icarus, Verilator, GHDL, NVC (each test skips if its tool is missing) | 12 |
| `cargo test` integration | `examples/dff/tests/sim.rs`, `examples/bus/tests/sim.rs` via `rivet::harness` (`--test-threads` shards) | Icarus | 11 + 25 |
| `cargo nextest` integration: one test per process, each with its own results directory, sharing one design build behind a lock; the terse listing nextest parses; no ignored tests | `.config/nextest.toml`, `examples/dff`, `examples/bus` | Icarus, `cargo-nextest` | 11 + 13 |
| Conformance on real simulators: cocotb's inertial-write cases, phases, timer precision, combinational visibility, edges, X before reset, 128-bit vectors, signed values, packed structs, string/integer/real variables, memory arrays, parameters, generate instances, force/release, discovery, plusargs, mass cancellation, HDL `$finish` | `examples/conformance` | Icarus, Verilator 5.020 and 5.036 | 21 per simulator |
| Kit against real RTL: AXI4-Lite, APB, Wishbone and Avalon register files, an AXI4 memory with bursts, an AXI-Stream skid buffer, a register slice into the kit's AXI4-Lite slave, a typed ALU through generated `Cmd`/`Op` types; random stimulus with coverage, golden traces per parameter set, checkers, `params`, `param_sets`, waveform windows, wall-clock limits | `examples/bus` | Icarus, Verilator 5.020 and 5.036 | 13 tests, 25 per simulator over two parameter sets |
| Kit (valid/ready, scoreboard) | `examples/fifo` | Icarus, Verilator | 3 per simulator |
| VHDL through VPI and VHPI | `examples/dff_vhdl` | GHDL, NVC | 2 per simulator |
| VHDL types: a record port, an enumeration and its literal names, a boolean, an integer, an unconstrained generic, a for-generate region | `examples/vhdl_types` | NVC (VHPI) or GHDL (VPI) | 6 tests: 5 pass and 1 skips on NVC, 2 pass and 4 skip on GHDL |
| FuseSoC and Edalize: the `rivet` tool backend generates `rivet.toml` from a core file and runs the dff example | `integrations/edalize`, `examples/dff/rivet-dff.core` | Icarus, Python | 1 flow |
| `rivet_py`: memory, RNG (bit-identical to Rust), scoreboard and coverage from Python; a cocotb test using them | `python/rivet_py/tests` | Python, cocotb, Icarus | 4 + 1 |

The skip on NVC and one of the four on GHDL is `list_children`, a hierarchy
dump that only runs with `RIVET_LIST_CHILDREN=1`. The other three GHDL skips
are the tests that need record members, which GHDL's VPI does not expose.

Run everything with:

```sh
cargo test --workspace --no-fail-fast          # unit, mock, CLI e2e, cargo-test harness
cargo nextest run -p example-dff -p example-bus # the same tests, one simulator per process
for e in dff fifo conformance bus; do
  target/debug/rivet run --sim icarus    -C examples/$e
  target/debug/rivet run --sim verilator -C examples/$e
done
target/debug/rivet run --sim icarus -C examples/bus -j 4 --cov-threshold 99
export PATH=/opt/nvc/bin:$PATH                 # wherever NVC is installed
for s in ghdl nvc; do
  target/debug/rivet run --sim $s -C examples/dff_vhdl
  target/debug/rivet run --sim $s -C examples/vhdl_types
done
pip install -e integrations/edalize && fusesoc --cores-root examples/dff run --no-export --tool rivet rivet:examples:dff
(cd python/rivet_py && maturin build --release -o dist && pip install dist/*.whl && cd tests && python -m unittest test_shim && cd cocotb_dff && python run.py)
```

Every run prints a `RIVET_RESULT` line with the counts above, so a claim in
this file can be checked by running the command next to it. Each CLI test
skips itself when its tool is not installed, so `cargo test` alone proves
less than the list above.

CI runs all of the above on Ubuntu 24.04 with apt Icarus 12, Verilator
5.020 and GHDL 4.1, and in the same job lists and runs the dff example under
`cargo nextest` and checks that a test-only edit still reaches a result in
under five seconds (`ci/loop-latency.py`). Separate jobs build Verilator
5.036 from source, build NVC 1.17.1 from a release tarball and run both VHDL
examples on it, run the Python integrations and the reference-model bridge,
build the book and resolve its links, check the package metadata, and pin
the 1.87 MSRV. The NVC numbers in this file were taken on 1.23; CI is what
says they still hold on 1.17.1.

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

The third expansion (`rivet new`, VHPI, runner parity, nextest) found five
more, all in the tooling and the tests themselves:

12. **Every Icarus end-to-end CLI test skipped silently, in CI too.** The
    probe for an installed tool ran `iverilog --version`, which Icarus
    rejects; the test read the failure as "not installed" and returned.
    The probe tries `-V` as well, and the Icarus CLI tests now run.
13. **Tests sharing an example directory raced.** With those tests running,
    two of them built and ran in one `sim_build` at the same time. Each
    example now has its own lock.
14. **`rivet bindgen --sim verilator` never saw its hierarchy dump.** It
    passed a relative path while the simulator ran in its own output
    directory, so the file was written somewhere else and bindgen failed.
    The path is absolute now.
15. **Bindgen output was not formatted**, so regenerating the committed,
    formatted bindings always produced a diff and the comparison could
    never pass. Bindgen runs rustfmt on what it writes.
16. **The bus CLI test cleared `coverage.json` before asserting on `cov
    report`.** A filtered run with no covergroups correctly leaves no file
    (defect 11), and the test that did so ran first. The order is fixed.
17. **The runner asked for `lib<name>.so` on macOS**, where Cargo writes
    `lib<name>.dylib`, so every Icarus, GHDL, NVC and commercial run died
    with "cannot copy ...: No such file or directory" before the simulator
    started. Found by the new macOS job; a unit test now pins the library
    name per platform.
18. **The Verilator shim asked the linker for `-lstdc++` on macOS**, which
    ships `libc++`, so every Verilator target failed to link there. The
    build script already distinguishes Mach-O for the `--start-group`
    question and now uses it here too.

Test-writing again caught timing-model mistakes (writes in the ReadOnly
phase after a checker or a `read_only().await`), which is why the immediate
checkers now return at the start of the next time step.

Bringing up the VHPI backend found five more simulator behaviours, four on
NVC and one on GHDL, each confirmed with a standalone C plugin against the
simulator before being fixed in the backend. They are recorded with the rest
of the per-simulator behaviour in
[`SIMULATOR-QUIRKS.md`](SIMULATOR-QUIRKS.md).

## Simulator differences the suite documents

| Property | Icarus 12 | Verilator 5.020 / 5.036 | GHDL 4.1 (VPI) | NVC 1.23 (VHPI) |
|---|---|---|---|---|
| Deposit readable back in the same ReadWrite phase | no | yes (applied as an immediate write at the flush) | not tested | not tested |
| `string` variables visible through VPI | no | yes | n/a | n/a |
| Packed struct / record members addressable by name | no (decoded from the vector) | no (decoded from the vector) | no (tests that need them skip) | yes (`vhpiSelectedNames`) |
| Enumeration literal names (`enum_literals`, `enum_name`) | no | no | no (an enumeration reads back as a vector) | yes (`OP_SUB` rather than 2) |
| Generate elements | pseudo-region `gen[i]` | pseudo-region `gen[i]` | the label resolves to its first element, so elements are found by scanning the enclosing scope | each element is its own region `label(i)`; the backend synthesises the array |
| Four-state values (X before reset) | yes | no (two-state) | yes | not tested |
| Force/release | yes | reported unsupported | not tested | `Capabilities` reports it (`vhpiForcePropagate`); not exercised by the suite |
| Generics/parameters by name | yes | yes (marked constant from the symbol table) | no (generics are not in the hierarchy) | yes (`vhpiGenericDecls`, marked constant) |
| Real and string parameters | yes | yes | n/a | n/a |
| Waveform control from the test | on/off (`$dumpon`/`$dumpoff`), one file per run | on/off and a new file per call | none (`--wave` covers the run) | none (`--wave` covers the run) |
| Internal scopes in the hierarchy | `$ivl_*`, `$unm_blk_*` (skipped by bindgen) | none | n/a | n/a |

The two VHDL columns are reproducible from one example:
`RIVET_LIST_CHILDREN=1 target/debug/rivet run --sim nvc -C examples/vhdl_types
--filter list_children` dumps what each simulator exposes. NVC reports the
record `cmd`, the enumeration `last_op`, the integers and the generics
`width` and `taps`; GHDL reports none of them and gives the enumeration and
the integer as vectors.

Everything else a particular simulator needs is in
[`SIMULATOR-QUIRKS.md`](SIMULATOR-QUIRKS.md), which also says which
workarounds have run on hardware and which are carried from cocotb's
catalogue unverified.

## Not covered

- Commercial simulators (no licence). Questa, Xcelium, VCS, Riviera and
  DSim have launch flows in the CLI and their quirks in the backends, but
  none of it has run; the conformance crate is what a licence holder should
  run first.
- FLI (Questa's VHDL interface). Mixed-language *routing* is covered
  against two composed mock backends, but no simulator hosting two
  languages in one process has run it, and `rivet-vpi`/`rivet-vhpi` do
  not implement `Backend::set_event_tag` yet, so a composite cannot be
  built from them today.
- Windows. On macOS, CI runs the unit tests and the dff, fifo and
  conformance examples on Icarus; Verilator is installed there but not
  exercised, and no VHDL simulator is.
- `Signal::slice` is covered against the mock only. It reads and writes
  through the whole vector, so it needs nothing from a backend.
- The Edalize backend is exercised through FuseSoC only; Edalize's flow
  API (`edalize.tools`) is not implemented.
