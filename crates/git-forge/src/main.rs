//! The main entrypoint for the CLI.

use clap::{CommandFactory, Parser};
use git_forge::cli::{Cli, Commands};
use std::path::PathBuf;
use std::process;

fn main() {
    if let Some(dir) = parse_generate_man_flag() {
        if let Err(e) = generate_man_page(dir) {
            eprintln!("Error: {e}");
            process::exit(1);
        }
        return;
    }

    let cli = Cli::parse();

    match cli.command {
        Commands::Issue { command } => git_forge_issues::exe::run(command),
        Commands::Review { command } => git_forge_review::exe::run(command),
        Commands::Release { command } => git_forge_release::exe::run(command),
    }
}

/// Check for `--generate-man <DIR>` before clap parses, so it doesn't
/// conflict with the (currently absent) required subcommand.
fn parse_generate_man_flag() -> Option<PathBuf> {
    let args: Vec<String> = std::env::args().collect();
    let pos = args.iter().position(|a| a == "--generate-man")?;
    let dir = args
        .get(pos + 1)
        .map(PathBuf::from)
        .unwrap_or_else(default_man_dir);
    Some(dir)
}

fn default_man_dir() -> PathBuf {
    std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            let home = std::env::var_os("HOME").expect("HOME is not set");
            PathBuf::from(home).join(".local/share")
        })
        .join("man")
}

fn generate_man_page(output_dir: PathBuf) -> Result<(), Box<dyn std::error::Error>> {
    let man1_dir = output_dir.join("man1");
    std::fs::create_dir_all(&man1_dir)?;

    let cmd = Cli::command();
    let man = clap_mangen::Man::new(cmd);
    let mut buffer = Vec::new();
    man.render(&mut buffer)?;

    let man_path = man1_dir.join("git-forge.1");
    std::fs::write(&man_path, &buffer)?;

    eprintln!("Wrote man page to {}", man_path.display());
    Ok(())
}
