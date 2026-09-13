# Closing the gap with cocotb

[`03-status.md`](03-status.md) records what Rivet does; this document is the
plan for everything cocotb still does better. It is written from the user's
side: each item is a reason someone would choose cocotb over Rivet today,
what it takes to remove that reason, how the removal is verified, and what
it costs. Engineering detail for the backend items lives in
[`04-remaining-work.md`](04-remaining-work.md); this is the ordering and the
exit criteria.

Rivet already matches cocotb's semantics and beats it on speed
([`../benchmarks.md`](../benchmarks.md)), so none of what follows is about
the core. The gaps are reach, ergonomics, and trust.

## The gaps, ranked

| # | Gap | Why it loses users | Phase |
|---|---|---|---|
| 1 | VHDL only through GHDL's VPI | every VHDL shop on NVC, Questa or Riviera is excluded | P0 |
| 2 | No install that does not need a Rust toolchain | `pip install cocotb` is one line | P0 |
| 3 | No user-facing documentation | design docs are not a tutorial | P0 |
| 4 | Runner gaps: regex filters, error kinds, shuffle, fixtures | cocotb regressions do not port one to one | P1 |
| 5 | Edit-to-result latency | Python has no compile step | P1 |
| 6 | Handle model: struct members, N-d arrays, slices, enum names | discovery-heavy testbenches hit walls | P1 |
| 7 | Commercial simulators unverified | nobody runs unverified code on a paid tool | P2 |
| 8 | Mixed-language designs | common in industry, `GPI_EXTRA` exists for it | P2 |
| 9 | Python ecosystem (numpy, pyuvm, cocotb extensions) | reference models already exist in Python | P2 |
| 10 | Interactive and GUI debugging | `GUI=1` then step in the simulator | P2 |
| 11 | Maturity: no releases, no soak, one benchmark run | a harness is infrastructure; trust is the product | P3 |
| 12 | macOS and Windows | a third of the desks | P3 |

## P0: reach and adoption

### 1. VHDL beyond GHDL

**Gap.** `rivet-vpi` reaches GHDL because GHDL exposes VPI. NVC, Questa,
Riviera and Xcelium expose VHDL only through VHPI (and Questa through FLI).
Generics are not reachable by name on GHDL, so even the supported path is
partial.

**Build.** `crates/rivet-vhpi`, per [`04-remaining-work.md`](04-remaining-work.md) §1:
sixteen `vhpi_*` bindings, a `VhpiBackend` porting cocotb's type
classification by base type with the subtype fallback, enum-content
heuristics for `std_logic`/`bit`/`boolean`/`character`, records through
`vhpiSelectedNames`, indexed names with the linear-scan fallback,
`vhpiDepositPropagate` writes, the four repetitive callback kinds, and the
three-stage root discovery. Export `vhpi_startup_routines` and a
`_bootstrap` symbol; add a `vhpi` feature to the facade so one `cdylib`
serves both PLIs. Runner: `rivet run --sim nvc` (`nvc -a/-e/-r --load=`).

**Verify.** `examples/dff_vhdl` on NVC with the tests written for GHDL, plus
a new `examples/vhdl_types` design with a record port, an unconstrained
array generic, an enum and a `for generate`, since that is where VHPI type
classification breaks. NVC in CI from a cached source build.

**Cost.** About 1500 lines, two to three days. Nothing needed from outside.

### 2. Install without a Rust toolchain

**Gap.** Using Rivet means installing Rust, then building the CLI from
source. cocotb is a wheel. A user evaluating both spends ten minutes before
the first simulation on one and thirty seconds on the other.

**Build.**
- Publish `rivet-core`, `rivet-macros`, `rivet-manifest`, `rivet-kit`,
  `rivet-vpi`, `rivet-verilator`, `rivet` and `rivet-cli` to crates.io;
  add `CHANGELOG.md` and a `cargo-release` config; make the workspace
  version the release version.
- A `release.yml` workflow: tag triggers cross-builds of `rivet` for
  `x86_64-unknown-linux-gnu`, `aarch64-unknown-linux-gnu`,
  `x86_64-apple-darwin`, `aarch64-apple-darwin`, publishes crates, attaches
  binaries and checksums to a GitHub release.
- A `pip install rivet-hdl` wheel: maturin-built, ships the `rivet` binary
  and `rivet_py`, with the console script on PATH. The testbench still
  needs `cargo` to compile, so the wheel is for the runner plus the Python
  shim, and the docs say so.
- `rivet new <name>` scaffolds a crate: `rivet.toml`, `hdl/`, a passing
  test, `build.rs`, `main.rs`, `tests/sim.rs`.

**Verify.** A CI job that, in a container with no Rust preinstalled, runs
the documented install line, then `rivet new demo && rivet run -C demo` on
Icarus. Release dry-run (`cargo publish --dry-run`) per crate on every push.

**Cost.** One day, plus the crates.io names.

### 3. Documentation a newcomer can use

**Gap.** `docs/` explains the design to someone building Rivet. cocotb ships
a tutorial, a quickstart per simulator, an API reference and a cookbook.

**Build.** An mdBook at `docs/book`, published to GitHub Pages by CI:
quickstart (dff in ten minutes), one page per simulator with the exact
prerequisites, the timing model with a phase diagram, writing drivers and
monitors, the bus models, random and coverage, debugging (task dumps, wall
timeouts, waveforms, logs), the CLI reference, and the cocotb migration
tables (moved from [`../migration.md`](../migration.md)). Rustdoc on every
public item with runnable examples where the mock backend allows, published
to docs.rs. A `cookbook/` of five complete testbenches: register file,
FIFO, AXI4-Lite peripheral, stream processor, CPU trace comparison.

**Verify.** `cargo test --doc` green; a link checker in CI; the quickstart
executed verbatim by the install job above.

**Cost.** Two days, and the biggest single lever on adoption.

## P1: parity of the daily loop

### 4. Regression-runner parity

**Gap** (against `regression.py`, analysis §5.3):

| cocotb | Rivet today |
|---|---|
| `re.search` filters | comma-separated substrings (`test.rs:68`) |
| `COCOTB_RANDOM_TEST_ORDER` shuffle | fixed order |
| `expect_error=(TypeError, ...)` | `expect_fail` bool only |
| `TestSuccess` raised anywhere ends the test as passed | only the main task returning `Ok` |
| `file`/`line` in `results.xml` | absent |
| pytest fixtures for setup and teardown | none |
| `pytest`, `pytest-xdist`, html reports | `cargo test`, `rivet -j`; nextest unproven |

**Build.** Regex filters behind a small `regex` dependency in the CLI and a
compiled-once matcher in the harness; `--shuffle[=seed]` using the run seed
so the order is reproducible; `#[rivet::test(expect_fail = "panicked at
'overflow'")]` matching the failure message, and `expect_timeout`;
`rivet::finish_test()` that unwinds every task with a pass; `file`/`line`
captured by the macro into `TestDesc` and emitted as JUnit attributes;
`#[rivet::fixture]` async functions that return a value (and optionally a
teardown closure) injected as test arguments, resolved per test, ordered
before the design handle; a `nextest.toml` plus a CI job proving
`cargo nextest run` lists and isolates Rivet tests correctly.

**Verify.** Extend `crates/rivet-mock/tests/regression.rs` with one case per
row; a CLI test asserting the JUnit attributes; the nextest job.

**Cost.** Two days. Fixtures are the only design question: they must not
reintroduce cocotb's suspend-a-sync-protocol problem, so they are plain
async functions run inside the same executor.

### 5. Edit-to-result latency (measured; the plan was wrong)

**Gap.** A one-character change to a testbench costs a Rust rebuild and,
on Verilator, a relink of the model. cocotb reruns immediately. This is the
complaint that will come back most often, and it is partly fixable.

**Measured first**, with `ci/loop-latency.py`, and the result changed the
plan. A test-only edit rebuilds in 0.4 s on Icarus and 0.6 s on Verilator;
an HDL edit costs 12.5 s on Verilator, all of it Verilator compiling the
design. The Rust rebuild was never the problem, so the hot-library loader
below is not worth building: it would shave a fraction of a second off the
fast case and nothing off the slow one. The numbers are in
[`../benchmarks.md`](../benchmarks.md) and the script is in CI.

**Not built, and why.**
- **Hot test libraries** (dlopen the test `cdylib` from the Verilator
  binary): would save about 0.4 s on an edit that already takes 0.6 s, and
  nothing on the 12.5 s HDL edit. Not built.
- Split `rivet-kit` generics that monomorphise per call site; `codegen-units`
  and `debug = 1` defaults in a documented `[profile.dev]` snippet emitted
  by `rivet new`; `-Zshare-generics` where stable.
- `rivet watch` already reruns on change; add `--filter` inheritance and
  keep the simulator process alive between runs where the backend allows
  (Icarus does not; Verilator with hot libraries does).

**Verify.** The recorded numbers in `docs/benchmarks.md` with a regression
threshold, and the same edit measured on cocotb for the comparison to be
honest.

**Cost.** Two to three days, mostly the loader.

### 6. Handle and value model

**Gap.** `Object::children` walks module relationships, so struct variables
have no members; `bindgen` emits one dimension of unpacked arrays; there is
no slicing on a handle; enum literal names are not reported; Verilog
`integer`/`time` land in the vector path.

**Build.** `vpiMember` iteration in the VPI backend and `vhpiSelectedNames`
in VHPI, both surfaced as `Object::children`; `bindgen` nested views for
packed and unpacked structs and N-dimensional arrays (`mem[i][j]`);
`Signal::slice(range)` returning a view that reads and writes a bit range
through the existing `aval`/`bval` buffers; enum literal names in
`ObjInfo` where the PLI reports them, used by the generated `TryFrom`;
`integer`/`time`/`real` mapped to their own accessors.

**Verify.** New cases in `examples/conformance` for each, with the
per-simulator outcome table extended; `bindgen` output for a design with a
2-D memory and a nested struct parsed back by `syn` in the CLI tests.

**Cost.** Two days.

## P2: reach into other people's flows

### 7. Commercial simulators

**Gap.** The quirks are implemented; none has ever executed. Status is
"code only" (`03-status.md` M7).

**Plan.** Unchanged from [`04-remaining-work.md`](04-remaining-work.md) §5:
land the runner flows for Questa, Xcelium, VCS, Riviera and DSim; ship
`SIMULATOR-QUIRKS.md` mapping every workaround to its cocotb source line and
its verification status; then verify on the two licences that are free,
Questa Intel FPGA Starter Edition (which also verifies VHPI and FLI) and
DSim Desktop. Each row of the conformance table moves to "verified" or
becomes a bug with a repro.

**Cost.** A day of work, then licence-bound.

### 8. Mixed language

**Gap.** One `Backend` per runtime, so a Verilog top with VHDL blocks is out
of reach.

**Build.** `CompositeBackend` routing `child_by_name` to the owning backend
with a tag in the handle's high bits, callbacks and time from the primary,
cross-boundary hand-off by fully qualified name.

**Verify.** Only possible on a simulator running both languages in one
process, so this lands behind §7's Questa licence. Build it small until then.

**Cost.** 300 lines once VHPI exists.

### 9. Python where Python is better

**Gap.** cocotb testbenches reach numpy, scipy, pyuvm, cocotb-coverage and
the cocotbext bus models. `python/rivet_py` sends Rivet's models *into*
Python; nothing goes the other way, so an existing Python reference model
cannot score a Rivet test.

**Build.** `rivet-py-model`: an optional feature embedding CPython through
PyO3 in the harness, exposing `PyModel` that implements the kit's `Model`
trait by calling a Python callable, with values marshalled as `int`,
`bytes` or numpy arrays. The GIL is taken only inside the call, so the hot
path is untouched, and the feature is off by default. Document the cost per
call in the benchmarks. Separately, a `uvm`-flavoured layer in the kit
(agent, sequencer, sequence, factory-free generics) for teams coming from
pyuvm, built on the existing `Driver`/`Monitor`/`Scoreboard`.

**Verify.** A `examples/pymodel` crate scoring an ALU against a numpy
model on Icarus, in the Python CI job; the layered example replacing the
hand-rolled tasks in `examples/fifo`.

**Cost.** Two days for the bridge, two for the layered kit.

### 10. Interactive debugging

**Gap.** cocotb can launch the simulator GUI (`GUI=1`) and let a user step
waveforms while the testbench runs. Rivet always runs batch.

**Build.** `rivet run --gui` mapping to `vsim -gui`, `xrun -gui`,
`dve`/`verdi`, and for the open tools `gtkwave` on the produced dump with a
saved view file; `--wave-open` to launch the viewer at the end of a failing
test only; document attaching `rust-gdb`/`lldb` to the simulator process
with the test library loaded, and `RUST_BACKTRACE` through the PLI
boundary.

**Cost.** Half a day for the open tools, the rest behind licences.

## P3: trust

### 11. Maturity

**Gap.** One benchmark run per number, no released version, no soak, no
external review of the FFI.

**Build.**
- Benchmarks repeated five times reporting median and spread, plus a real
  design (PicoRV32 running a program for 100k cycles) so the harness
  overhead is stated as a fraction of a real simulation; a CI benchmark job
  with a regression threshold.
- A nightly soak: the bus example for an hour under `-j 8` with a random
  seed per run, failing on any nondeterminism; ASAN and Valgrind runs of
  the Icarus flow to catch PLI lifetime bugs.
- `cargo fuzz` targets for `LogicVec` parsing, `$readmemh` parsing and the
  typedef parser.
- A security review of `rivet-vpi` and `rivet-verilator`, which trust
  simulator-reported widths inside `unsafe`; then an external reviewer who
  knows cocotb on the timing model, the write-buffering rules, and the
  quirk table.
- Semver policy, MSRV in CI, `#[non_exhaustive]` on public enums, and the
  1.0 criteria written down.

**Cost.** Three days spread over the other phases.

### 12. Platforms

**Gap.** Linux only.

**Build.** macOS: `-undefined dynamic_lookup` for the `cdylib`, the Verilator
link-group flags (`-all_load` instead of `--start-group`), Homebrew Icarus
and Verilator in a CI job. Windows: import libraries from `.def` files per
simulator, as cocotb does; deferred until asked for.

**Cost.** A day for macOS.

## Order and milestones

| Milestone | Contents | Exit criteria |
|---|---|---|
| N0 | §2 release and install, §3 book and quickstart | A user with no Rust installed runs a simulation from published artefacts by following one page |
| N1 | §1 VHPI on NVC, §6 handle model | `examples/vhdl_types` passes on NVC and GHDL; conformance covers members, N-d arrays, slices |
| N2 | §4 runner parity, §5 iteration speed | Every row of the runner table has a test; a test-only edit rebuilds in under two seconds on Verilator |
| N3 | §7 runner flows and quirk doc, §9 Python bridge, §10 open-tool GUI | `rivet run --sim questa -C examples/conformance` is one command for a licence holder; numpy model example green |
| N4 | §11 assurance, §12 macOS, §8 mixed language | Benchmarks with spread on a real design; nightly soak green for a week; 1.0 criteria met |

N0 is first because every later item is worth more once people can install
the thing. N1 is the largest single population currently locked out. N2 is
what decides whether people stay after the first week.

## Deliberately not planned

- **FLI.** 2000 lines of Questa-specific C for a path VHPI already covers;
  build only if Questa's VHPI proves too slow in practice
  ([`04-remaining-work.md`](04-remaining-work.md) §3).
- **A pytest plugin.** cocotb's two-session socket protocol exists because
  pytest is synchronous; Rivet's harness is async and libtest-compatible,
  so the equivalent is nextest support, not pytest.
- **Running cocotb testbenches on Rivet's backends.** A GPI-compatible
  shim would mean embedding CPython and reimplementing cocotb's object
  model, which is the cost cocotb pays and Rivet exists to avoid.
  `rivet_py` plus the migration guide covers incremental moves instead.
- **Windows**, until someone asks.
