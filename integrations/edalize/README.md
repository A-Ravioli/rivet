# edalize-rivet

An [Edalize](https://github.com/olofk/edalize) tool backend for Rivet, so
[FuseSoC](https://github.com/olofk/fusesoc) cores can run their Rust
testbenches with `fusesoc run --tool rivet`.

```sh
pip install -e integrations/edalize      # registers the `rivet` tool
cargo install --path crates/rivet-cli    # or have target/debug/rivet on PATH
fusesoc --cores-root examples/dff run --no-export --tool rivet rivet:examples:dff
```

The backend writes a `rivet.toml` into the work root from the EDAM
description (top level, HDL files, include directories, parameters and
defines) and runs `rivet run --manifest <work root>/rivet.toml -C <crate>`.

Tool options (`tools: rivet:` in the core file, or `--rivet_*` overrides):

| option | meaning |
|---|---|
| `simulator` | `icarus` (default), `verilator`, or `ghdl` |
| `crate` | directory of the Rust test crate; default: the directory of a listed `Cargo.toml` (file type `user`) |
| `rivet_options` | extra `rivet run` arguments, e.g. `["-j", "4", "--waves"]` |
| `rivet` | the `rivet` executable (default: `rivet` on PATH, else `RIVET_BIN`) |

Plusargs (`--plusarg` / core `parameters` of type `plusarg`) are passed to
the simulator after `--`. Verilog parameters and VHDL generics become
`[design.params]`; defines become `[design.defines]`.

Use `--no-export` when the test crate is part of a Cargo workspace (its
`Cargo.toml` refers to the workspace by relative path). A standalone crate
with absolute dependency paths can be exported as usual.
