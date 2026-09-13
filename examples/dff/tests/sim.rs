//! `cargo test -p example-dff` entry point; see `rivet::harness`.
use example_dff as _;

fn main() -> std::process::ExitCode {
    rivet::harness::main()
}
