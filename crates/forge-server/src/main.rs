//! Forge sync daemon — coordinates import/export with remote adapters.

use std::path::PathBuf;
use std::time::Duration;

use anyhow::Result;
use forge_github::GitHubAdapter;
use forge_github::config::discover_github_configs;
use git_forge::sync::RemoteSync;
use git2::Repository;

fn main() -> Result<()> {
    let args = parse_args()?;
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

struct Args {
    repo: PathBuf,
    poll_interval: u64,
    once: bool,
}

fn parse_args() -> Result<Args> {
    let mut args = std::env::args().skip(1);
    let mut repo = PathBuf::from(".");
    let mut poll_interval = 60u64;
    let mut once = false;

    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--repo" => {
                repo = PathBuf::from(
                    args.next()
                        .ok_or_else(|| anyhow::anyhow!("--repo requires a value"))?,
                );
            }
            "--poll-interval" => {
                let val = args
                    .next()
                    .ok_or_else(|| anyhow::anyhow!("--poll-interval requires a value"))?;
                poll_interval = val.parse()?;
            }
            "--once" => once = true,
            other => anyhow::bail!("unknown argument: {other}"),
        }
    }

    Ok(Args {
        repo,
        poll_interval,
        once,
    })
}
