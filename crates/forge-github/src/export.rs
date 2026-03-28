//! forge → GitHub export functions.

use anyhow::Result;
use git_forge::Store;
use git_forge::refs::ISSUE_INDEX;
use git2::Repository;

use crate::client::GitHubClient;
use crate::config::GitHubSyncConfig;
use crate::state::{load_sync_state, lookup_by_forge_oid, save_sync_state};
use git_forge::sync::SyncReport;

/// Export locally-created issues to GitHub.
///
/// Issues already recorded in the sync state are skipped. Each exported issue
/// writes `issues/<n> → <oid>` to sync state and `<sigil><n> → <oid>` to
/// the issue index.
///
/// # Errors
/// Returns an error if the GitHub API call fails or a git operation fails.
pub async fn export_issues(
    repo: &Repository,
    cfg: &GitHubSyncConfig,
    client: &impl GitHubClient,
) -> Result<SyncReport> {
    let mut state = load_sync_state(repo, &cfg.owner, &cfg.repo)?;

    let store = Store::new(repo);
    let issues = store.list_issues()?;
    let mut report = SyncReport::default();

    for issue in &issues {
        if lookup_by_forge_oid(&state, "issues", &issue.oid).is_some() {
            report.skipped += 1;
            continue;
        }

        let labels: Vec<String> = issue.labels.clone();
        let assignees: Vec<String> = issue.assignees.clone();

        match client
            .create_issue(
                &cfg.owner,
                &cfg.repo,
                &issue.title,
                &issue.body,
                &labels,
                &assignees,
            )
            .await
        {
            Ok(number) => {
                let sigil = cfg.sigils.get("issue").map_or("GH#", String::as_str);
                let display_id = format!("{sigil}{number}");
                if let Err(e) = store.write_display_id(ISSUE_INDEX, &display_id, &issue.oid) {
                    eprintln!("forge: failed to write display ID for issue {number}: {e}");
                    report.failed += 1;
                    continue;
                }

                let state_key = format!("issues/{number}");
                state.insert(state_key, issue.oid.clone());
                report.exported += 1;
            }
            Err(e) => {
                eprintln!("forge: failed to export issue {}: {e}", issue.oid);
                report.failed += 1;
            }
        }
    }

    save_sync_state(repo, &cfg.owner, &cfg.repo, &state)?;
    Ok(report)
}
