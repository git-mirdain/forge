//! Forge sync daemon — coordinates import/export with remote adapters.

use std::path::PathBuf;
use std::process::Command;
use std::time::Duration;

use anyhow::Result;
use clap::Parser;
use forge_github::GitHubAdapter;
use forge_github::config::{SyncScope, discover_github_configs};
use git_forge::comment::rebuild_comments_index;
use git_forge::sync::RemoteSync;
use git2::Repository;

/// Forge sync daemon — watches refs and coordinates GitHub sync.
#[derive(Parser, Debug)]
#[command(version)]
struct Args {
    /// Path to the git repository (default: current directory).
    #[arg(long, default_value = ".")]
    repo: PathBuf,

    /// Seconds between sync polls.
    #[arg(long, default_value_t = 60u64)]
    poll_interval: u64,

    /// Run a single sync pass and exit.
    #[arg(long)]
    once: bool,

    /// Git remote for pushing/pulling forge refs (default: origin).
    #[arg(long, default_value = "origin")]
    remote: String,
}

fn main() -> Result<()> {
    let args = Args::parse();
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
    let repo_path = repo
        .workdir()
        .or_else(|| repo.path().parent())
        .unwrap_or(repo.path());

    loop {
        // Fetch remote forge refs before import/export.
        fetch_forge_refs(repo_path, &args.remote);

        for adapter in adapters {
            sync_one(repo, adapter).await?;
        }

        if let Err(e) = rebuild_comments_index(repo) {
            eprintln!("forge-server: index rebuild failed: {e:#}");
        }

        // Push forge refs after import/export.
        push_forge_refs(repo_path, &args.remote);

        if args.once {
            return Ok(());
        }

        tokio::time::sleep(Duration::from_secs(args.poll_interval)).await;
    }
}

/// Fetch `refs/forge/*` from the remote. Warns on failure.
fn fetch_forge_refs(repo_path: &std::path::Path, remote: &str) {
    let refspec = "+refs/forge/*:refs/forge/*";
    match Command::new("git")
        .args(["-C", &repo_path.display().to_string()])
        .args(["fetch", remote, refspec])
        .output()
    {
        Ok(output) if output.status.success() => {
            eprintln!("forge-server: fetched forge refs from {remote}");
        }
        Ok(output) => {
            let stderr = String::from_utf8_lossy(&output.stderr);
            eprintln!("forge-server: fetch forge refs failed: {stderr}");
        }
        Err(e) => {
            eprintln!("forge-server: fetch forge refs failed: {e}");
        }
    }
}

/// Push `refs/forge/*` to the remote. Warns on failure.
fn push_forge_refs(repo_path: &std::path::Path, remote: &str) {
    let refspec = "refs/forge/*:refs/forge/*";
    match Command::new("git")
        .args(["-C", &repo_path.display().to_string()])
        .args(["push", remote, refspec])
        .output()
    {
        Ok(output) if output.status.success() => {
            eprintln!("forge-server: pushed forge refs to {remote}");
        }
        Ok(output) => {
            let stderr = String::from_utf8_lossy(&output.stderr);
            eprintln!("forge-server: push forge refs failed: {stderr}");
        }
        Err(e) => {
            eprintln!("forge-server: push forge refs failed: {e}");
        }
    }
}

async fn sync_one(repo: &Repository, adapter: &GitHubAdapter) -> Result<()> {
    let cfg = &adapter.config;
    let label = format!("{}/{}", cfg.owner, cfg.repo);
    let sync_issues = cfg.sync.contains(&SyncScope::Issues);
    let sync_reviews = cfg.sync.contains(&SyncScope::Reviews);

    if sync_issues && sync_reviews {
        let import = adapter.import_all(repo).await?;
        eprintln!(
            "forge-server: import {label}: imported={} skipped={} failed={}",
            import.imported, import.skipped, import.failed,
        );
        let export = adapter.export_all(repo).await?;
        eprintln!(
            "forge-server: export {label}: exported={} skipped={} failed={} unexportable={}",
            export.exported, export.skipped, export.failed, export.unexportable,
        );
    } else if sync_issues {
        let import = adapter.import_issues(repo).await?;
        eprintln!(
            "forge-server: import issues {label}: imported={} skipped={} failed={}",
            import.imported, import.skipped, import.failed,
        );
        let export = adapter.export_issues(repo).await?;
        eprintln!(
            "forge-server: export issues {label}: exported={} skipped={} failed={} unexportable={}",
            export.exported, export.skipped, export.failed, export.unexportable,
        );
    } else if sync_reviews {
        let import = adapter.import_reviews(repo).await?;
        eprintln!(
            "forge-server: import reviews {label}: imported={} skipped={} failed={}",
            import.imported, import.skipped, import.failed,
        );
        let export = adapter.export_reviews(repo).await?;
        eprintln!(
            "forge-server: export reviews {label}: exported={} skipped={} failed={} unexportable={}",
            export.exported, export.skipped, export.failed, export.unexportable,
        );
    }

    Ok(())
}
