//! Verilator entry point: the model is linked in by `build.rs`.

// Link the test library so its `#[rivet::test]` registrations are present.
use example_dff as _;

fn main() {
    rivet::verilator::main()
}
