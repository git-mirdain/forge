//! Forge sync daemon library — coordinates import/export with remote adapters.

mod pidfile;

use std::ffi::OsStr;
use std::path::Path;
use std::process::Command;
use std::time::Duration;

use anyhow::Result;
use forge_github::GitHubAdapter;
use forge_github::config::{SyncScope, discover_github_configs};
use git_forge::comment::rebuild_comments_index;
use git_forge::sync::RemoteSync;
use git2::Repository;

/// Configuration for the sync daemon (no clap dependency).
pub struct ServerConfig {
    /// Seconds between sync polls.
    pub poll_interval: u64,
    /// Run a single sync pass and exit.
    pub once: bool,
    /// Git remote for pushing/pulling forge refs.
    pub remote: String,
    /// Disable fetching/pushing forge refs to the remote.
    pub no_sync_refs: bool,
}

/// Ref prefixes to sync. Excludes `refs/forge/config` to avoid leaking
/// tokens or other sensitive config to the remote.
pub const SYNC_REF_PREFIXES: &[&str] = &[
    "refs/forge/issues/*:refs/forge/issues/*",
    "refs/forge/reviews/*:refs/forge/reviews/*",
    "refs/forge/comments/*:refs/forge/comments/*",
    "refs/forge/contributors/*:refs/forge/contributors/*",
    "refs/forge/index/*:refs/forge/index/*",
];

/// Discover GitHub adapters from the repository's forge config.
///
/// # Errors
/// Returns an error if config discovery fails.
pub fn discover_adapters(repo: &Repository) -> Result<Vec<GitHubAdapter>> {
    let configs = discover_github_configs(repo)?;
    if configs.is_empty() {
        eprintln!("forge-server: no GitHub configs found in refs/forge/config");
    }
    configs
        .into_iter()
        .map(|c| GitHubAdapter::new(c).map_err(Into::into))
        .collect()
}

/// Run the sync loop with the given configuration.
///
/// Initializes a single-threaded tokio runtime and blocks on the async
/// sync loop. This is the main entry point for both the standalone binary
/// and the `forge server start` subcommand.
///
/// # Errors
/// Returns an error if the tokio runtime fails to build or the sync loop
/// encounters an unrecoverable error.
pub fn run(repo: &Repository, config: &ServerConfig) -> Result<()> {
    let _pid_guard = pidfile::PidGuard::acquire(repo.path())?;
    // `git2::Repository` is `!Sync`, so a multi-threaded runtime would be unsound.
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?
        .block_on(run_loop(repo, config))
}

async fn run_loop(repo: &Repository, config: &ServerConfig) -> Result<()> {
    let repo_path = repo
        .workdir()
        .or_else(|| repo.path().parent())
        .unwrap_or(repo.path());

    loop {
        if !config.no_sync_refs {
            fetch_forge_refs(repo_path, &config.remote);
            // TODO: the libgit2 `Repository` handle may cache ref state; freshly
            // fetched refs are visible because we fetch via the `git` CLI, but
            // in-process caches (e.g. packfile index) could theoretically go stale.
        }

        let adapters = match discover_adapters(repo) {
            Ok(a) => a,
            Err(e) => {
                eprintln!("forge-server: adapter discovery failed: {e:#}");
                Vec::new()
            }
        };

        for adapter in &adapters {
            let label = format!("{}/{}", adapter.config.owner, adapter.config.repo);
            if let Err(e) = sync_one(repo, adapter).await {
                eprintln!("forge-server: sync failed for {label}: {e:#}");
            }
        }

        if let Err(e) = rebuild_comments_index(repo) {
            eprintln!("forge-server: index rebuild failed: {e:#}");
        }

        if !config.no_sync_refs {
            push_forge_refs(repo_path, &config.remote);
        }

        if config.once {
            return Ok(());
        }

        tokio::select! {
            () = tokio::time::sleep(Duration::from_secs(config.poll_interval)) => {}
            _ = tokio::signal::ctrl_c() => {
                eprintln!("forge-server: received SIGINT, shutting down");
                return Ok(());
            }
        }
    }
}

/// Fetch forge refs from the remote (non-force to avoid destroying local work).
pub fn fetch_forge_refs(repo_path: &Path, remote: &str) {
    for refspec in SYNC_REF_PREFIXES {
        match Command::new("git")
            .args([OsStr::new("-C"), repo_path.as_os_str()])
            .args(["fetch", remote, refspec])
            .output()
        {
            Ok(output) if output.status.success() => {}
            Ok(output) => {
                let stderr = String::from_utf8_lossy(&output.stderr);
                eprintln!("forge-server: fetch {refspec} failed (refs may be stale): {stderr}");
            }
            Err(e) => {
                eprintln!("forge-server: fetch {refspec} failed (refs may be stale): {e}");
            }
        }
    }
    eprintln!("forge-server: fetched forge refs from {remote}");
}

/// Push forge refs to the remote.
pub fn push_forge_refs(repo_path: &Path, remote: &str) {
    for refspec in SYNC_REF_PREFIXES {
        match Command::new("git")
            .args([OsStr::new("-C"), repo_path.as_os_str()])
            .args(["push", remote, refspec])
            .output()
        {
            Ok(output) if output.status.success() => {}
            Ok(output) => {
                let stderr = String::from_utf8_lossy(&output.stderr);
                eprintln!("forge-server: push {refspec} failed: {stderr}");
            }
            Err(e) => {
                eprintln!("forge-server: push {refspec} failed: {e}");
            }
        }
    }
    eprintln!("forge-server: pushed forge refs to {remote}");
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
