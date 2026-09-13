//! `cargo test -p example-bus` entry point; see `rivet::harness`.
use example_bus as _;

fn main() -> std::process::ExitCode {
    rivet::harness::main()
}
