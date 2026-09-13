# Changelog

All notable changes to Rivet are recorded here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/) and the project
uses [semantic versioning](https://semver.org/spec/v2.0.0.html).

Until 1.0, minor versions may contain breaking changes; they will always be
listed under "Changed" with the migration in one line.

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

## [0.1.0] - unreleased

First public version. Design documents, the harness, the CLI, the kit, the
Icarus, Verilator, GHDL and mock backends, examples and benchmarks.
