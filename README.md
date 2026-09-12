# rivet

Faster cocotb-style hardware verification in Rust.

Rivet is a native harness for driving HDL simulators (Verilator, Icarus,
GHDL, NVC, Questa, Xcelium, VCS, Riviera, DSim) from Rust `async` testbenches.
It keeps cocotb's timing model and vocabulary and replaces the embedded
Python interpreter, string-typed values, and dual build flows.

Status: design phase. See [`docs/design/`](docs/design/README.md).
