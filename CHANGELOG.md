# Changelog

All notable changes to Rivet are recorded here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/) and the project
uses [semantic versioning](https://semver.org/spec/v2.0.0.html).

Until 1.0, minor versions may contain breaking changes; they will always be
listed under "Changed" with the migration in one line.

## Versioning policy

- **Public API.** Everything reachable from the `rivet` facade, plus
  `rivet-kit` and the `rivet.toml` schema. The CLI's flags and its exit
  codes are public too: a script that runs `rivet run` is a user.
- **Not public.** `rivet-core`'s `runtime` internals, the `Backend` trait's
  exact shape, the FFI crates' bindings, and the on-disk layout of
  `sim_build`. A backend living outside this repository should expect to
  track `rivet-core` version for version.
- **Enums.** Enums a user *reads* — `ObjKind`, `Error`, `BackendError`,
  `Outcome`, `Phase` — are `#[non_exhaustive]`, so a new variant is not a
  breaking change; match them with a wildcard arm. Enums a *backend
  implements against* — `CbKind`, `Action`, `Value`, `WaveCmd` — are
  deliberately exhaustive, because a backend that silently ignores a new
  callback kind is worse than one that fails to compile. Adding a variant
  to those is a breaking change.
- **MSRV.** Currently 1.87, checked in CI. Raising it is a minor-version
  change, never a patch.
- **Simulator support.** Adding a simulator is a minor version. Dropping
  one, or changing which one `rivet run` picks by default, is breaking.

### What 1.0 requires

1. The conformance suite passing on at least one commercial simulator
   (Questa, Xcelium or VCS), not just Icarus, Verilator, GHDL and NVC.
2. `docs/SIMULATOR-QUIRKS.md` with no "code only" rows for a supported
   simulator: every workaround Rivet carries has been observed, not
   inferred from cocotb.
3. A mixed-language design running end to end on a real simulator, which
   means `set_event_tag` in `rivet-vpi` and `rivet-vhpi`.
4. The nightly soak green for a month, including the ASAN and Valgrind
   runs of the Icarus flow.
5. An external review of the `unsafe` in `rivet-vpi`, `rivet-vhpi` and
   `rivet-verilator`, all of which trust simulator-reported widths.
6. A real project's testbench ported from cocotb by someone who did not
   write Rivet, with the friction recorded in `docs/migration.md`.

## [Unreleased]

### Added

- **Testbenches in Python.** `@rivet.test` on an `async def`, driven by
  Rivet's own executor rather than an interpreter-owned scheduler: a
  trigger is a simulator callback and a coroutine is one more task on the
  executor, entered once per `await`. `rivet run --python` needs no crate
  and no `cargo`; a `[python]` section in `rivet.toml` names the modules to
  import. Measured against cocotb 2.1 on the same design, simulator and
  machine: 7.8x on an edge per cycle, 32.3x on per-cycle value traffic,
  3.5x with a hundred tasks awaiting every edge — and 1.3x-1.6x slower than
  the Rust API, which is the honest cost of the interpreter. Clocks and the
  kit are native tasks that cost a Python testbench nothing, and
  `await clk.rising_edge(n=...)` waits many cycles for one resume.
  See `docs/python.md`; sources in `python/rivet`.
- `rivet.mock`: Rivet's mock simulator driven from Python, so a Python
  testbench (and this bridge's own test suite) runs with no simulator
  installed.
- `rivet-core::test::TestSpec` and `run_regression_specs`, so tests
  discovered at run time go through the same regression loop, seeding,
  timeouts and `results.xml` as `#[rivet::test]`.
- VHPI backend (`rivet-vhpi`) and the `nvc` simulator flow, so VHDL designs
  run on NVC as well as on GHDL through VPI.
- `rivet new` scaffolds a testbench crate that runs without further edits.
- Signal slicing (`Signal::slice`), struct members through the PLI, and enum
  literal names in `ObjInfo`.
- `bindgen` emits multi-dimensional unpacked arrays and nested struct views.
- Regression parity with cocotb: regular-expression filters, reproducible
  `--shuffle`, `expect_fail = "message"`, `expect_timeout`,
  `rivet::runtime::finish_test()`, and `file`/`line` attributes in
  `results.xml`.
- `#[rivet::fixture]` for async setup and teardown shared between tests.
- Launch flows for Questa, Xcelium, VCS, Riviera and DSim, with
  `docs/SIMULATOR-QUIRKS.md` recording the verification status of every
  per-simulator workaround.
- `--gui` and waveform-viewer launch for the open tools.
- An optional Python reference-model bridge (`rivet-kit` feature `python`).
- `CompositeBackend`: one testbench over two procedural interfaces, for
  mixed-language designs. Routing and tagging are tested against two
  composed mock backends; no simulator hosting both languages has run it.
- The user-facing book under `docs/book`, published by CI.
- Release workflow: crates.io publishing and cross-built binaries.

### Changed

- **Published names.** The crates are `rivet-hdl`, `rivet-hdl-core`,
  `rivet-hdl-cli` and so on; the wheel is `rivet-hdl`. Plain `rivet` was
  taken on both crates.io and PyPI. What you write is unchanged: each
  crate keeps its old library name, so `use rivet::prelude::*` and
  `use rivet_core::…` still compile, and the Python import is still
  `rivet`. Depend on it as `rivet = { package = "rivet-hdl", version = … }`
  to keep the short key, which is what `rivet new` now generates.
- `pip install rivet-hdl` is now enough on its own: the wheel carries the
  `rivet` CLI and the PLI plugin alongside the bindings, so a Python
  testbench needs no cargo and no Rust toolchain. One wheel per platform
  and Python minor version, because the plugin links libpython.
- `rivet run` gained `--plugin` and `--python-tests`.
- The release workflow publishes to PyPI, and to crates.io it now also
  publishes `rivet-hdl-vhpi`, which was missing from the list.
- `rivet-vpi` and `rivet-vhpi` gained a default `startup-table` feature.
  It is on unless turned off, so nothing changes for a test crate; the
  Python plugin turns it off because a shared object can export only one
  PLI startup table and it exports its own.
- Minimum supported Rust version is 1.87, checked in CI.
- `ObjKind`, `Error`, `BackendError`, `Outcome` and `Phase` are
  `#[non_exhaustive]`; match them with a wildcard arm.

## [0.1.0] - unreleased

First public version. Design documents, the harness, the CLI, the kit, the
Icarus, Verilator, GHDL and mock backends, examples and benchmarks.
