//! Verilator entry point: the model is linked in by `build.rs`.
use example_bus as _;

fn main() {
    rivet::verilator::main()
}
