# Testing

What is tested, where, and what the expanded suite found.

## Layers

| Layer | Where | Needs | Count |
|---|---|---|---|
| Values (`LogicVec`, `Logic`), time units and rounding, Fx hash, result selection and XML escaping | `crates/rivet-core/src/*.rs` unit tests | nothing | 24 |
| Manifest parsing and lookup | `crates/rivet-manifest` | nothing | 2 |
| `#[rivet::test]` attribute parsing | `crates/rivet-macros` | nothing | 1 |
| Bindgen code generation (parsed back with `syn`), libtest-style selection, `results.xml` summary, build hashing, argument parsing | `crates/rivet-cli/src/*.rs` unit tests | nothing | 6 |
| Timing model and executor semantics: edge, ReadWrite, ReadOnly, NextTimeStep phases; deposit buffering and dedup; trusted-inertial mode; force/release; timers with equal deadlines; drop-deregistration of timers and edge waiters; multiple waiters per edge; scopes; join handles; child panics; `yield`/`join`/`with_timeout`; `Queue`, `Lock`, `Event`; clock builder options; signal and hierarchy API; constant writes; determinism across runs | `crates/rivet-mock/tests/{core,semantics}.rs` | nothing (mock simulator) | 35 |
| The regression runner: stage/module/name ordering, skip, `expect_fail` both ways, timeouts, invalid timeouts, task and child panics, sim-time accounting, filters, `results.xml` round trip, hierarchy dump JSON, HDL-side finish | `crates/rivet-mock/tests/regression.rs` | nothing | 5 |
| CLI end to end: run, filter, exit codes, HDL `$finish` reported as failure, waveform output, `bindgen` output parses, error paths, Verilator and GHDL flows | `crates/rivet-cli/tests/cli.rs` | Icarus, Verilator, GHDL (each test skips if its tool is missing) | 6 |
| `cargo test` integration | `examples/dff/tests/sim.rs` via `rivet::harness` | Icarus | 10 |
| Conformance on real simulators: cocotb's inertial-write cases, phases, timer precision, combinational visibility, edges, X before reset, 128-bit vectors, signed values, packed structs, string/integer/real variables, memory arrays, parameters (int, real, string), generate instances, force/release, discovery, plusargs, mass cancellation, HDL `$finish` | `examples/conformance` | Icarus, Verilator 5.020 and 5.036 | 21 per simulator |
| Kit (valid/ready, scoreboard) | `examples/fifo` | Icarus, Verilator | 3 per simulator |
| VHDL | `examples/dff_vhdl` | GHDL | 2 |

Run everything with:

```sh
cargo test --workspace --no-fail-fast          # unit, mock, CLI e2e, cargo-test harness
for e in dff fifo conformance; do
  target/debug/rivet run --sim icarus    -C examples/$e
  target/debug/rivet run --sim verilator -C examples/$e
done
target/debug/rivet run --sim ghdl -C examples/dff_vhdl
```

CI runs all of the above on Ubuntu 24.04 with apt Icarus 12, Verilator
5.020 and GHDL 4.1, plus a second job with Verilator 5.036 built from
source.

## What the expanded suite found

Writing these tests surfaced the following defects, all fixed in the same
change:

1. **A bad `timeout` expression aborted the whole regression.** The guard
   covered evaluating the expression but not converting it to simulator
   steps, which panicked inside the regression task. Now both happen under
   the guard and only that test fails (`suite::bad_timeout` in
   `regression.rs`).
2. **Writing a constant was silently dropped.** The check lived in the
   backend and ran at ReadWrite flush time, where it could only log. It now
   happens at the call site and the test fails
   (`writing_a_constant_fails`).
3. **Signal paths disagreed between simulators.** Verilator reports
   top-level ports as `TOP.clk` while the root is `conformance`; paths are
   now built from parent path plus name, as cocotb does, and generate
   elements get `parent[i]` (`discovery`, `generate_instances`).
4. **Verilator direct access could write the wrong storage.** A top-level
   port exists twice in a Verilated model: the real storage under `TOP`
   and a per-module alias the model refreshes every evaluation. Writing
   the alias made a harness-driven clock never toggle. Root-level names
   now resolve the `TOP` scope first (`writes_on_timer_seen_on_edge` on
   Verilator).
5. **Verilator parameters were writable.** Its VPI types them like
   variables; the symbol table's `isParam` is now used to mark them
   constant (`parameters` on Verilator).
6. **`IntoLogicVec` was missing from the prelude.**

Test-writing also caught three of my own timing-model mistakes (writes in
the ReadOnly phase, counting the clock's timer, the mock clock's first
edge at t=0); each panicked exactly as the model says it should, which is
the behaviour under test.

## Simulator differences the suite documents

Logged rather than asserted, because they are properties of the tools:

| Property | Icarus 12 | Verilator 5.020 / 5.036 | GHDL 4.1 |
|---|---|---|---|
| Deposit readable back in the same ReadWrite phase | no | yes (applied as an immediate write at the flush) | not tested |
| `string` variables visible through VPI | no | yes | n/a |
| Packed struct members addressable by name | no | no | n/a |
| Four-state values (X before reset) | yes | no (two-state) | yes |
| Force/release | yes | reported unsupported | not tested |
| Generics/parameters by name | yes | yes (marked constant from the symbol table) | no |
| Real and string parameters | yes | yes | n/a |

## Not covered

- Commercial simulators (no licence); the conformance crate is what a
  licence holder should run.
- VHPI (no backend yet); GHDL runs through VPI.
- Mixed-language designs.
- Wall-clock hangs: a testbench that waits for an edge that never comes
  runs until its simulated-time `timeout`, and forever without one. A CLI
  wall-clock limit is on the remaining-work list.
- macOS and Windows.
