//! The `forge` CLI.

use clap::Parser as _;

fn main() {
    let cli = git_forge::cli::Cli::parse();
    if let Err(e) = git_forge::exe::Executor::discover().and_then(|exe| exe.run(&cli)) {
        eprintln!("error: {e}");
        std::process::exit(1);
    }
}
