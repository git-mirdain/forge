//! Forge sync daemon — coordinates import/export with remote adapters.

use std::path::PathBuf;
use std::time::Duration;

use anyhow::Result;
use facet::Facet;
use figue::{self as args, FigueBuiltins};
use forge_github::GitHubAdapter;
use forge_github::config::discover_github_configs;
use git_forge::sync::RemoteSync;
use git2::Repository;

/// Forge sync daemon — watches refs and coordinates GitHub sync.
#[derive(Facet, Debug)]
struct Args {
    /// Path to the git repository (default: current directory).
    #[facet(args::named, default = PathBuf::from("."))]
    repo: PathBuf,

    /// Seconds between sync polls.
    #[facet(args::named, default = 60u64)]
    poll_interval: u64,

    /// Run a single sync pass and exit.
    #[facet(args::named)]
    once: bool,

    /// Built-in flags (--help, --version, --completions).
    #[facet(flatten)]
    builtins: FigueBuiltins,
}

fn main() -> Result<()> {
    let args: Args = figue::from_std_args().unwrap();
    let repo = Repository::discover(&args.repo)?;

    let configs = discover_github_configs(&repo)?;
    if configs.is_empty() {
        eprintln!("forge-server: no GitHub configs found in refs/forge/config");
    }

    let adapters: Vec<GitHubAdapter> = configs.into_iter().map(GitHubAdapter::new).collect();

    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?
        .block_on(run(&repo, &adapters, &args))
}

async fn run(repo: &Repository, adapters: &[GitHubAdapter], args: &Args) -> Result<()> {
    loop {
        for adapter in adapters {
            sync_one(repo, adapter).await?;
        }

        if args.once {
            return Ok(());
        }

        tokio::time::sleep(Duration::from_secs(args.poll_interval)).await;
    }
}

async fn sync_one(repo: &Repository, adapter: &GitHubAdapter) -> Result<()> {
    let cfg = &adapter.config;
    let label = format!("{}/{}", cfg.owner, cfg.repo);

    let import = adapter.import_issues(repo).await?;
    eprintln!(
        "forge-server: import {label}: imported={} skipped={} failed={}",
        import.imported, import.skipped, import.failed,
    );

    let export = adapter.export_issues(repo).await?;
    eprintln!(
        "forge-server: export {label}: exported={} skipped={} failed={}",
        export.exported, export.skipped, export.failed,
    );

    Ok(())
}
