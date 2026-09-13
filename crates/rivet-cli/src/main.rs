fn main() -> std::process::ExitCode {
    rivet_cli::main_with_args(std::env::args().skip(1))
}
