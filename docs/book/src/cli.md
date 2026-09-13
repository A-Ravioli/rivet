# CLI reference

```text
usage: rivet <new|run|build|watch|bindgen|cov|clean> [options] [-- sim args]
```

Everything after a bare `--` is passed to the simulator command unchanged.

## Commands

| Command | What it does |
|---|---|
| `rivet new <name>` | scaffold a testbench crate that runs as generated |
| `rivet run` | build the crate and the design, run the tests, report |
| `rivet build` | build the crate and the design, run nothing |
| `rivet watch` | run, then rerun whenever a source file changes |
| `rivet bindgen` | run the design once to dump its hierarchy, then write typed bindings |
| `rivet cov report [files...]` | merge `coverage.json` files and report |
| `rivet clean` | remove `sim_build` |

### `rivet new <name>`

Writes a standalone crate at `<dir>/<name>`, where `<dir>` is the current
directory or whatever `-C` names. `rivet new .` scaffolds into the current
directory and takes the crate name from it. Non-alphanumeric characters in
the name become underscores, and a leading digit gets an underscore prefix,
so `my-tb` becomes the crate `my_tb`.

It writes `Cargo.toml`, `rivet.toml`, `build.rs`, `hdl/counter.sv`,
`src/lib.rs` with two tests, `src/main.rs`, `tests/sim.rs`, `.gitignore` and
`README.md`, then prints:

```text
rivet: created /tmp/counter-tb
rivet: next: cd counter-tb && rivet run --sim icarus
```

It refuses to write into a directory that already has a `Cargo.toml`. The
generated `Cargo.toml` carries an empty `[workspace]` table, so the crate
does not join a surrounding workspace. `--path <dir>` points its dependencies
at a Rivet checkout instead of crates.io.

### `rivet run`

Builds the test crate with Cargo, compiles the design for the chosen
simulator, starts the simulator with the harness loaded, and reads back
`results.json` and `results.xml`. With `[design.param_sets]` in the manifest
it does that once per set unless `--param-set` narrows it. With `-j N` it
asks the harness for its test list, splits it round-robin into shards, and
runs one simulator process per shard.

Output artefacts land in `sim_build/<sim>/`, and under
`sim_build/<sim>/<param_set>/` and `.../shard<i>/` when those apply.

### `rivet build`

The same build steps, stopping before the simulator runs. Useful for warming
a cache or checking that a design compiles.

### `rivet watch`

Runs, then waits for a change and runs again. It watches the crate's `src`,
`tests` and `golden` directories, `Cargo.toml`, `build.rs`, `rivet.toml`, and
every source and include directory the manifest lists. Stop it with Ctrl-C.

### `rivet bindgen`

Runs the design once with `RIVET_DUMP_HIERARCHY` set, which makes the harness
walk the hierarchy and write `sim_build/<sim>/hierarchy.json` instead of
running tests, then generates Rust bindings from that dump and the HDL
sources. The output goes to `src/dut.rs` unless `-o` says otherwise.

The generated module has one struct per instance with a field per signal,
`Vec<Signal>` for arrays, a `hierarchy()` method, and Rust enums and
bit-layout structs for the `typedef enum` and `typedef struct packed`
declarations found in the sources.

### `rivet cov report`

Merges `coverage.json` files and prints the table. With no positional files
it uses `sim_build/<sim>/coverage.json` from the crate. `-o` writes the
merged JSON, and `--threshold` sets the exit status.

### `rivet clean`

Removes the `sim_build` directory in the current directory, or in the directory `-C` names.

## Options

| Option | Argument | Default | Meaning |
|---|---|---|---|
| `--sim` | `icarus`, `verilator`, `ghdl` | `RIVET_SIM`, else `icarus` | simulator |
| `-p`, `--package` | crate name | the crate in the current directory | which workspace member to build |
| `-C` | directory | the current directory | change to this directory first |
| `--release` | none | off | build the harness in release mode |
| `--filter`, `-k` | `a,b` | none | run only tests whose name or `module::name` contains one of these |
| `-j`, `--jobs` | `n` | 1 | run the tests in `n` simulator processes; `-j4` also works |
| `--param-set` | name | every set in the manifest | run only this `[design.param_sets]` entry |
| `--waves` | none | off | dump waveforms |
| `--waves-per-test` | none | off | one waveform file per test (Verilator); implies `--waves` |
| `--seed` | integer, decimal or `0x` hex | drawn from the clock | random seed, printed by every run for replay |
| `--wall-timeout` | seconds | none | per-test wall-clock limit |
| `--log` | `error`, `warn`, `info`, `debug`, `trace` | `info` | log level |
| `--log-format` | `text`, `json` | `text` | log record format |
| `--log-dir` | directory | `sim_build/<sim>/logs` | per-test log files |
| `--no-log-dir` | none | off | write no per-test log files |
| `--cov-threshold`, `--threshold` | percentage | none | fail the run below this functional coverage |
| `--update-golden` | none | off | rewrite golden trace files from this run |
| `--manifest` | path | `rivet.toml` next to or above the crate | design description |
| `--path` | directory | none | `new`: depend on a Rivet checkout instead of crates.io |
| `-o`, `--out` | file | `src/dut.rs` | `bindgen` output; also the merged file for `cov report` |
| `-v`, `--verbose` | none | off | print the commands being run |
| `-h`, `--help` | none | | print usage and exit 2 |
| `--` | | | pass the rest to the simulator |

An unknown option, a missing numeric argument, or no command at all prints
the usage text and exits 2.

## Exit codes

| Code | Meaning |
|---|---|
| 0 | everything passed |
| 1 | a test failed, or coverage was below `--cov-threshold` |
| 2 | a usage error, or the run could not be set up (no `rivet.toml`, no package, a build failure) |

A simulator process aborted by the watchdog exits 3 itself. `rivet run` then
finds no `results.json` for it and reports that as a setup failure, so the
`rivet` command exits 2.

## Examples

```sh
rivet run --sim icarus    -C examples/dff
rivet run --sim verilator -C examples/dff
rivet run --sim ghdl      -C examples/dff_vhdl
rivet run --sim icarus -C examples/dff --filter counter --waves --log debug
rivet run --sim icarus -C examples/bus -j 4 --cov-threshold 95 --seed 42
rivet run --sim icarus -C examples/bus --param-set init5 --update-golden
rivet build --sim verilator -C examples/bus
rivet watch --sim verilator -C examples/bus
rivet bindgen --sim icarus -C examples/dff          # writes src/dut.rs
rivet cov report -C examples/bus --threshold 100
rivet clean -C examples/bus
rivet run --sim icarus -C examples/dff -- +my_plusarg=3
```

## `cargo test` options

With the `harness` feature and a `harness = false` test target,
`rivet::harness::main()` accepts the libtest options `cargo test` passes:

| Option | Meaning |
|---|---|
| positional filter | substring of `module::name` |
| `--exact` | filters must equal `module::name` or the bare test name |
| `--skip <pattern>` | exclude matching tests |
| `--list` | list tests without a simulator |
| `--test-threads <n>` | simulator processes, the same as `-j` |
| `--format json` | libtest JSON output |
| `--nocapture`, `--ignored`, `--include-ignored`, `--show-output`, `-q` | accepted and ignored |

```sh
cargo test -p example-dff -- --list
cargo test -p example-dff
RIVET_SIM=verilator cargo test -p example-dff -- counter
cargo test -p example-bus -- --test-threads 2
```

## Environment variables

Variables the CLI sets for each simulator process, and which a test or an
embedding harness can set directly:

| Variable | Set by | Meaning |
|---|---|---|
| `RIVET_SIM` | the user | default simulator for `rivet` and `cargo test` |
| `RIVET_MANIFEST` | `--manifest` | path to `rivet.toml`; also read by the Verilator `build.rs` |
| `RIVET_SEED` | `--seed` | base seed for the run |
| `RIVET_TEST_FILTER` | `--filter` | comma-separated substrings selecting tests |
| `RIVET_TEST_SELECT` | `-j N` | exact `module::name` list for one shard |
| `RIVET_PARAM_SET` | `--param-set`, and per set | the manifest parameter set this process runs; readable as `rivet::test::param_set()` |
| `RIVET_WALL_TIMEOUT` | `--wall-timeout` | per-test wall-clock limit in seconds |
| `RIVET_WATCHDOG_GRACE` | the user | extra seconds before the watchdog aborts; default 5 |
| `RIVET_LOG` | `--log` | log level |
| `RIVET_LOG_FORMAT` | `--log-format` | `text` or `json` |
| `RIVET_LOG_TIME_UNIT` | the user | `ns` (default), `ps`, `us`, `step` |
| `RIVET_LOG_DIR` | `--log-dir` | per-test log file directory |
| `RIVET_WAVES` | `--waves`, `--waves-per-test` | `1` or `per-test` |
| `RIVET_RESULTS_FILE` | the CLI | where to write `results.xml` |
| `RIVET_RESULTS_JSON` | the CLI | where to write `results.json` |
| `RIVET_COVERAGE_FILE` | the CLI | where to write `coverage.json` |
| `RIVET_GOLDEN_DIR` | the CLI | golden trace directory, `<crate>/golden` |
| `RIVET_UPDATE_GOLDEN` | `--update-golden` | `1` rewrites golden files instead of comparing |
| `RIVET_TOPLEVEL` | the CLI | the top-level name the harness binds to |
| `RIVET_DUMP_HIERARCHY` | `rivet bindgen` | dump the hierarchy to this path instead of running tests |
| `RIVET_LIST_TESTS` | `rivet run -j N` | write the test list here and exit, instead of running |
| `RIVET_JOBS` | the user | overrides `--test-threads` under `cargo test` |
| `RIVET_WATCH_ONCE` | the user | `rivet watch` runs once and returns, for tests |
| `RIVET_TRUST_INERTIAL_WRITES` | the user | `1` makes the VPI backend trust inertial writes; ignored on Verilator |
| `RIVET_VERILATOR_DIRECT` | the user | `0` forces the VPI value path instead of direct model access |
| `VERILATOR` | the user | the `verilator` binary the build helper runs |
| `RIVET_BENCH_N` | the user | cycle count for `examples/bench` |
| `RIVET_BIN` | the user | the `rivet` binary the Edalize backend runs |
