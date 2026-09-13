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

- Minimum supported Rust version is 1.87, checked in CI.
- `ObjKind`, `Error`, `BackendError`, `Outcome` and `Phase` are
  `#[non_exhaustive]`; match them with a wildcard arm.

## [0.1.0] - unreleased

First public version. Design documents, the harness, the CLI, the kit, the
Icarus, Verilator, GHDL and mock backends, examples and benchmarks.
