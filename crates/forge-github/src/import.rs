//! GitHub → forge import functions.

use anyhow::{Context, Result};
use git_forge::Store;
use git2::Repository;

use crate::client::{fetch_issues, make_client};
use crate::config::GitHubSyncConfig;
use crate::state::{load_sync_state, save_sync_state};

/// Summary of a single import run.
#[derive(Debug, Default)]
pub struct SyncReport {
    /// Number of entities newly imported.
    pub imported: usize,
    /// Number of entities exported (unused in import-only runs).
    pub exported: usize,
    /// Number of entities skipped (already present in sync state).
    pub skipped: usize,
    /// Number of entities that failed to import.
    pub failed: usize,
}

/// Import all GitHub issues from `cfg.owner`/`cfg.repo` into the local forge store.
///
/// Issues already recorded in the sync state are skipped. Each new issue stores
/// the GitHub issue URL as a `source` field for provenance.
///
/// # Errors
/// Returns an error if the GitHub API call fails or a git operation fails.
pub async fn import_issues(repo: &Repository, cfg: &GitHubSyncConfig) -> Result<SyncReport> {
    let client = make_client(cfg.token.as_deref())?;
    let mut state = load_sync_state(repo, &cfg.owner, &cfg.repo)?;
    let issues = fetch_issues(&client, &cfg.owner, &cfg.repo).await?;

    let store = Store::new(repo);
    let mut report = SyncReport::default();

    for issue in &issues {
        let state_key = format!("issues/{}", issue.number);
        if state.contains_key(&state_key) {
            report.skipped += 1;
            continue;
        }

        let display_id = format!("{}{}", cfg.sigil, issue.number);
        let login = &issue.user.login;
        let email = format!("{login}@users.noreply.github.com");
        let source = format!(
            "https://github.com/{}/{}/issues/{}",
            cfg.owner, cfg.repo, issue.number
        );

        let author = git2::Signature::now(login, &email)
            .with_context(|| format!("invalid git signature for issue {}", issue.number))?;

        let labels: Vec<&str> = issue.labels.iter().map(|l| l.name.as_str()).collect();
        let assignees: Vec<&str> = issue.assignees.iter().map(|a| a.login.as_str()).collect();
        let body = issue.body.as_deref().unwrap_or("");

        match store.create_issue_imported(
            &issue.title,
            body,
            &labels,
            &assignees,
            &display_id,
            &author,
            &source,
        ) {
            Ok(created) => {
                state.insert(state_key, created.oid.clone());
                report.imported += 1;
            }
            Err(e) => {
                eprintln!("forge: failed to import issue {}: {e}", issue.number);
                report.failed += 1;
            }
        }
    }

    save_sync_state(repo, &cfg.owner, &cfg.repo, &state)?;
    Ok(report)
}
