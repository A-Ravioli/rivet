# Simulators

`rivet run --sim <name>` accepts `icarus`, `verilator` and `ghdl`. Anything
else is rejected:

```text
rivet: error: unsupported simulator "nvc" (icarus, verilator, ghdl)
```

Every simulator reads the same `rivet.toml`. The `[design]` section is shared;
each simulator has its own `[sim.<name>]` section.

## Shared manifest keys

```toml
[design]
top = "dff"                    # required: top-level module or entity
sources = ["hdl/dff.sv"]       # paths relative to rivet.toml
includes = ["hdl/include"]     # include directories
timescale = "1ns/1ps"          # optional
language = "verilog"           # "verilog" (default) or "vhdl"

[design.defines]               # preprocessor defines, string values
SIM = "1"

[design.params]                # top-level parameter overrides, string values
WIDTH = "8"

[design.param_sets.w16]        # optional: rebuild and rerun the design per set
WIDTH = "16"
```

| Key | Type | Meaning |
|---|---|---|
| `design.top` | string, required | top-level module or entity name |
| `design.sources` | list of paths | HDL sources, relative to the manifest directory |
| `design.includes` | list of paths | include directories |
| `design.defines` | table of strings | preprocessor defines |
| `design.params` | table of strings | top-level parameter or generic overrides |
| `design.timescale` | string | time unit and precision |
| `design.language` | string | `verilog` (default) or `vhdl` |
| `design.param_sets.<name>` | table of strings | overrides applied over `design.params`; `rivet run` builds and runs once per set |

`rivet.toml` is found next to the crate or in an ancestor directory, or named
explicitly with `--manifest`.

## Icarus Verilog

**Prerequisites.** Icarus Verilog 11 or newer. CI runs the Ubuntu 24.04
package, which is 12.0. The `iverilog` and `vvp` commands must be on `PATH`.

**Manifest keys.**

```toml
[sim.icarus]
args = ["-g2012"]        # extra iverilog arguments
run_args = ["+foo=1"]    # extra vvp arguments, including plusargs
```

`trace` and `timing` are ignored for Icarus.

**What `rivet run --sim icarus` does.** It builds the crate's `cdylib`, then:

```text
iverilog -o sim_build/icarus/<top>.vvp -s <top> -g2012 \
         -I<include>... -D<k>=<v>... -P<top>.<k>=<v>... <args> <sources>
vvp -M sim_build/icarus -m <lib_name> sim_build/icarus/<top>.vvp <run_args>
```

`-g2012` is added only if `args` contains no `-g` option of its own. A
`timescale` writes `+timescale+<value>` into `sim_build/icarus/cmds.f` and
passes it with `-f`. The build is content-hashed into `build.hash`, so an
unchanged design is not recompiled.

**Waveforms.** Icarus needs `$dumpvars` inside the design, so `--waves`
generates a companion module `sim_build/icarus/rivet_dump.sv`, adds it as a
second top level, and writes `sim_build/icarus/<top>.fst`. The generated
module toggles `$dumpon`/`$dumpoff` from a register the harness drives, which
is how `rivet::waves::on()` and `off()` work. There is one file per run.

## Verilator

**Prerequisites.** Verilator 5.x with a C++17 compiler. Rivet is verified on
5.020 and 5.036. Set the `VERILATOR` environment variable to use a binary
that is not `verilator` on `PATH`.

**Manifest keys.**

```toml
[sim.verilator]
args = ["-Wno-WIDTH", "-Wno-UNUSEDSIGNAL"]   # extra verilator arguments
trace = true                                  # --trace --trace-structs
timing = false                                # --timing, for delays and events
run_args = []                                 # extra arguments to the built binary
```

**Crate requirements.** Verilator is not a PLI host: Rivet owns `main`. The
crate needs a binary target and a `build.rs`:

```toml
[[bin]]
name = "dff_verilator"
path = "src/main.rs"
required-features = ["verilator"]

[build-dependencies]
rivet-verilator = { workspace = true }

[features]
verilator = ["rivet/verilator"]
```

```rust
fn main() {
    if std::env::var_os("CARGO_FEATURE_VERILATOR").is_some() {
        rivet_verilator::Build::from_manifest().build();
    }
}
```

`build.rs` reads the same `rivet.toml` and runs:

```text
verilator --cc --vpi --public-flat-rw --build -Mdir <objdir> \
          --top-module <top> --prefix <prefix> \
          -CFLAGS "-fPIC -std=gnu++17" -CFLAGS -O1|-O2 \
          [--trace|--trace-fst --trace-structs] [--timing] \
          +incdir+<include>... +define+<k>=<v>... -G<k>=<v>... <args> <sources>
```

The C++ optimisation level follows Cargo's profile: `-O1` for a debug build,
`-O2` for `--release`. Verilator's own default is `-Os`, and the benchmarks
attribute 10 to 18% to this change alone. The Verilator version is recorded
in a stamp file, so changing the tool forces a rebuild, and each parameter
set gets its own object directory.

**Values.** Rivet reads and writes signals directly in the model's storage
through `VerilatedScope::varFind`, using VPI only for value-change detection.
Set `RIVET_VERILATOR_DIRECT=0` to force the VPI path.

**Waveforms.** `--waves` passes `--trace --trace-file <run_dir>/<top>.vcd`
to the built binary. `--waves-per-test` writes one file per test. Tracing
must be enabled at build time with `trace = true` in the manifest.

## GHDL

**Prerequisites.** GHDL 4.x with VPI support. CI runs the Ubuntu 24.04
package, which is 4.1. The design must declare `language = "vhdl"`.

**Manifest keys.**

```toml
[design]
top = "dff"
language = "vhdl"
sources = ["hdl/dff.vhd"]

[sim.ghdl]
args = ["--std=08"]      # passed to -a and -e; only --std and -P reach -r
run_args = []            # extra arguments to ghdl -r
```

`trace` and `timing` are ignored for GHDL.

**What `rivet run --sim ghdl` does.**

```text
ghdl -a --workdir=sim_build/ghdl --std=08 <args> <sources>
ghdl -e --workdir=sim_build/ghdl --std=08 <args> <top>
ghdl -r --workdir=sim_build/ghdl --std=08 <top> --vpi=<lib>.so \
        -g<k>=<v>... [--wave=<run_dir>/<top>.ghw] <run_args>
```

`--std=08` is added only if `args` contains no `--std` option. GHDL runs
through VPI, not VHPI. Generics are passed on the command line with `-g`, but
they are not reachable by name from the testbench on GHDL.

**Waveforms.** `--waves` passes `--wave=<top>.ghw`, which covers the whole
run. The waveform control functions in `rivet::waves` have no effect on GHDL.

## NVC

NVC is not supported. There is no VHPI backend, and `rivet run --sim nvc`
fails with `unsupported simulator "nvc"`. NVC exposes VHPI rather than VPI,
so it cannot use the VPI path that GHDL uses.

The plan is written up in `docs/design/04-remaining-work.md` §1: a
`crates/rivet-vhpi` backend, and a runner that would issue

```text
nvc -a --std=08 <sources>
nvc -e <top>
nvc -r <top> --load=<lib>.so
```

Nothing of that exists in the code yet. Treat the section above as a plan,
not a feature.

## Behaviour differences

These are the differences the test suites pin down, from `docs/testing.md`.

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

Two consequences worth planning for:

- A test that asserts X before reset must check
  `rivet::runtime::caps().four_state` first, as `examples/dff` does in
  `x_propagates_before_reset`. Verilator is two-state.
- Inertial writes are buffered by the runtime and flushed at ReadWrite on
  Icarus and Verilator; GHDL is trusted to apply them itself. See
  [The timing model](timing-model.md).

## Other simulators

Questa, Xcelium, VCS, Riviera, DSim and CVC have quirk handling in
`rivet-vpi` ported from cocotb, but none of them has ever been run, and there
is no runner support in the CLI. `examples/conformance` is the suite a
licence holder should run first.
