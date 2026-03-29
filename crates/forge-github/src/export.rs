//! forge → GitHub export functions.

use anyhow::Result;
use git_forge::Store;
use git_forge::comment::{issue_comment_ref, list_comments};
use git_forge::refs::ISSUE_INDEX;
use git_forge::sync::SyncReport;
use git2::Repository;

use crate::client::GitHubClient;
use crate::config::GitHubSyncConfig;
use crate::state::{load_sync_state, lookup_by_forge_oid, save_sync_state};

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

    // Save issue state before exporting comments so newly exported issues
    // are visible to export_issue_comments.
    save_sync_state(repo, &cfg.owner, &cfg.repo, &state)?;

    let all_issues = store.list_issues()?;
    for issue in &all_issues {
        let comment_report = export_issue_comments(repo, cfg, client, &issue.oid).await?;
        report.exported += comment_report.exported;
        report.skipped += comment_report.skipped;
        report.failed += comment_report.failed;
    }

    Ok(report)
}

/// Export locally-created issue comments to GitHub.
///
/// Comments already recorded in the sync state are skipped.
///
/// # Errors
/// Returns an error if the GitHub API call fails or a git operation fails.
pub async fn export_issue_comments(
    repo: &Repository,
    cfg: &GitHubSyncConfig,
    client: &impl GitHubClient,
    forge_issue_oid: &str,
) -> Result<SyncReport> {
    let mut state = load_sync_state(repo, &cfg.owner, &cfg.repo)?;

    let Some(github_number) = lookup_by_forge_oid(&state, "issues", forge_issue_oid) else {
        return Ok(SyncReport::default());
    };

    let ref_name = issue_comment_ref(forge_issue_oid);
    let Ok(comments) = list_comments(repo, &ref_name) else {
        return Ok(SyncReport::default());
    };

    let mut report = SyncReport::default();

    for comment in &comments {
        if lookup_by_forge_oid(&state, "comments", &comment.oid).is_some() {
            report.skipped += 1;
            continue;
        }

        match client
            .create_issue_comment(&cfg.owner, &cfg.repo, github_number, &comment.body)
            .await
        {
            Ok(github_comment_id) => {
                state.insert(format!("comments/{github_comment_id}"), comment.oid.clone());
                report.exported += 1;
            }
            Err(e) => {
                eprintln!(
                    "forge: failed to export comment {} on issue {forge_issue_oid}: {e}",
                    comment.oid
                );
                report.failed += 1;
            }
        }
    }

    save_sync_state(repo, &cfg.owner, &cfg.repo, &state)?;
    Ok(report)
}
