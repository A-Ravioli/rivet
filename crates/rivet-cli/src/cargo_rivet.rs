//! `cargo rivet ...`: the same command line as `rivet`, invoked through
//! cargo (`cargo install rivet-hdl-cli` puts both on the path).

fn main() -> std::process::ExitCode {
    let mut args = std::env::args().skip(1).peekable();
    // cargo passes the subcommand name as the first argument.
    if args.peek().map(|a| a == "rivet").unwrap_or(false) {
        args.next();
    }
    rivet_cli::main_with_args(args)
}
