//! forge → GitHub export functions.

use std::collections::HashMap;

use anyhow::Result;
use git_forge::Store;
use git_forge::comment::{find_threads_by_object, list_thread_comments};
use git_forge::refs::{ISSUE_INDEX, ISSUE_PREFIX, REVIEW_INDEX, REVIEW_PREFIX, walk_tree};
use git_forge::sync::SyncReport;
use git2::Repository;

use crate::client::GitHubClient;
use crate::config::GitHubSyncConfig;
use crate::state::{load_sync_state, lookup_by_forge_oid, save_sync_state};

/// Try to find the path of a blob in a commit's tree.
///
/// `commit_oid` is the commit (or tree) to walk, `blob_oid` is the blob to find.
/// Returns `None` if the object can't be resolved or the blob isn't found.
fn resolve_blob_path(repo: &Repository, commit_oid: &str, blob_oid: &str) -> Option<String> {
    let oid = git2::Oid::from_str(commit_oid).ok()?;
    let obj = repo.find_object(oid, None).ok()?;
    let tree = match obj.kind() {
        Some(git2::ObjectType::Commit) => repo.find_commit(oid).ok()?.tree().ok()?,
        Some(git2::ObjectType::Tree) => repo.find_tree(oid).ok()?,
        _ => return None,
    };
    let mut files = Vec::new();
    walk_tree(repo, &tree, "", &mut files);
    files
        .into_iter()
        .find(|(_, oid)| oid == blob_oid)
        .map(|(path, _)| path)
}

/// Returns `true` if the string looks like a branch name rather than a raw OID.
fn is_branch_name(s: &str) -> bool {
    let len = s.len();
    if (len == 40 || len == 64) && s.chars().all(|c| c.is_ascii_hexdigit()) {
        return false;
    }
    !s.is_empty()
}

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
                    eprintln!("forge: failed to write display ID for issue {number}: {e:#}");
                    report.failed += 1;
                    continue;
                }

                let source_url = format!(
                    "https://github.com/{}/{}/issues/{number}",
                    cfg.owner, cfg.repo,
                );
                if let Err(e) = store.write_source_url(ISSUE_PREFIX, &issue.oid, &source_url) {
                    eprintln!("forge: failed to write source URL for issue {number}: {e:#}");
                }

                let state_key = format!("issues/{number}");
                state.insert(state_key, issue.oid.clone());
                report.exported += 1;
            }
            Err(e) => {
                eprintln!("forge: failed to export issue {}: {e:#}", issue.oid);
                report.failed += 1;
            }
        }
    }

    for issue in &issues {
        let comment_report =
            export_issue_comments_with_state(repo, cfg, client, &issue.oid, &mut state).await?;
        report.exported += comment_report.exported;
        report.skipped += comment_report.skipped;
        report.failed += comment_report.failed;
    }

    save_sync_state(repo, &cfg.owner, &cfg.repo, &state)?;
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
    let report =
        export_issue_comments_with_state(repo, cfg, client, forge_issue_oid, &mut state).await?;
    save_sync_state(repo, &cfg.owner, &cfg.repo, &state)?;
    Ok(report)
}

async fn export_issue_comments_with_state(
    repo: &Repository,
    cfg: &GitHubSyncConfig,
    client: &impl GitHubClient,
    forge_issue_oid: &str,
    state: &mut HashMap<String, String>,
) -> Result<SyncReport> {
    let Some(github_number) = lookup_by_forge_oid(state, "issues", forge_issue_oid) else {
        return Ok(SyncReport::default());
    };

    let thread_ids = find_threads_by_object(repo, forge_issue_oid)?;
    let mut report = SyncReport::default();

    for thread_id in &thread_ids {
        let comments = list_thread_comments(repo, thread_id)?;
        for comment in &comments {
            if lookup_by_forge_oid(state, "comments", &comment.oid).is_some() {
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
                        "forge: failed to export comment {} on issue {forge_issue_oid}: {e:#}",
                        comment.oid
                    );
                    report.failed += 1;
                }
            }
        }
    }

    Ok(report)
}

/// Export locally-created reviews to GitHub as pull requests.
///
/// Reviews already recorded in the sync state are skipped. Each exported review
/// writes `reviews/<n> → <oid>` to sync state and `<sigil><n> → <oid>` to
/// the review index.
///
/// # Errors
/// Returns an error if the GitHub API call fails or a git operation fails.
pub async fn export_reviews(
    repo: &Repository,
    cfg: &GitHubSyncConfig,
    client: &impl GitHubClient,
) -> Result<SyncReport> {
    let mut state = load_sync_state(repo, &cfg.owner, &cfg.repo)?;

    let store = Store::new(repo);
    let reviews = store.list_reviews()?;
    let mut report = SyncReport::default();

    for review in &reviews {
        if lookup_by_forge_oid(&state, "reviews", &review.oid).is_some() {
            report.skipped += 1;
            continue;
        }

        // A GitHub PR requires a branch name as the head ref.  Reviews
        // created with --path or --head (without --ref) store only an OID,
        // which GitHub cannot accept.
        let Some(ref source_ref) = review.source_ref else {
            eprintln!(
                "forge: skipping review {} — no --ref; \
                 cannot create a GitHub PR without a branch name",
                review.oid,
            );
            report.unexportable += 1;
            continue;
        };

        if !is_branch_name(source_ref) {
            eprintln!(
                "forge: skipping review {} — source ref \"{source_ref}\" looks like \
                 a raw OID, not a branch name",
                review.oid,
            );
            report.unexportable += 1;
            continue;
        }

        let base_ref = review.target.base.as_deref().unwrap_or("main");

        match client
            .create_pull(
                &cfg.owner,
                &cfg.repo,
                &review.title,
                &review.body,
                source_ref,
                base_ref,
            )
            .await
        {
            Ok(number) => {
                let sigil = cfg.sigils.get("review").map_or("GH#", String::as_str);
                let display_id = format!("{sigil}{number}");
                if let Err(e) = store.write_display_id(REVIEW_INDEX, &display_id, &review.oid) {
                    eprintln!("forge: failed to write display ID for review {number}: {e:#}");
                    report.failed += 1;
                    continue;
                }

                let source_url = format!(
                    "https://github.com/{}/{}/pull/{number}",
                    cfg.owner, cfg.repo,
                );
                if let Err(e) = store.write_source_url(REVIEW_PREFIX, &review.oid, &source_url) {
                    eprintln!("forge: failed to write source URL for review {number}: {e:#}");
                }

                let state_key = format!("reviews/{number}");
                state.insert(state_key, review.oid.clone());
                report.exported += 1;
            }
            Err(e) => {
                eprintln!("forge: failed to export review {}: {e:#}", review.oid);
                report.failed += 1;
            }
        }
    }

    // Export review comments for each synced review.
    for review in &reviews {
        let comment_report =
            export_review_comments_with_state(repo, cfg, client, &review.oid, &mut state).await?;
        report.exported += comment_report.exported;
        report.skipped += comment_report.skipped;
        report.failed += comment_report.failed;
    }

    save_sync_state(repo, &cfg.owner, &cfg.repo, &state)?;
    Ok(report)
}

/// Export locally-created review comments to GitHub.
///
/// # Errors
/// Returns an error if the GitHub API call fails or a git operation fails.
pub async fn export_review_comments(
    repo: &Repository,
    cfg: &GitHubSyncConfig,
    client: &impl GitHubClient,
    forge_review_oid: &str,
) -> Result<SyncReport> {
    let mut state = load_sync_state(repo, &cfg.owner, &cfg.repo)?;
    let report =
        export_review_comments_with_state(repo, cfg, client, forge_review_oid, &mut state).await?;
    save_sync_state(repo, &cfg.owner, &cfg.repo, &state)?;
    Ok(report)
}

async fn export_review_comments_with_state(
    repo: &Repository,
    cfg: &GitHubSyncConfig,
    client: &impl GitHubClient,
    forge_review_oid: &str,
    state: &mut HashMap<String, String>,
) -> Result<SyncReport> {
    let Some(github_number) = lookup_by_forge_oid(state, "reviews", forge_review_oid) else {
        return Ok(SyncReport::default());
    };

    // Load the review to get the head commit for blob-path resolution.
    let store = Store::new(repo);
    let review_head: Option<String> = store
        .get_review(forge_review_oid)
        .ok()
        .map(|r| r.target.head.clone());

    let thread_ids = find_threads_by_object(repo, forge_review_oid)?;
    let mut report = SyncReport::default();

    for thread_id in &thread_ids {
        let comments = list_thread_comments(repo, thread_id)?;
        for comment in &comments {
            if lookup_by_forge_oid(state, "comments", &comment.oid).is_some() {
                report.skipped += 1;
                continue;
            }

            // For review comments with anchor data, use create_review_comment.
            // For plain comments (no anchor), fall back to issue comment API on the PR.
            if let Some(ref anchor) = comment.anchor {
                let line = anchor.start_line.unwrap_or(1);

                // Resolve blob OID to a file path by walking the review's head commit tree.
                // Use anchor.oid as the blob OID; commit is the review's target head.
                let path = review_head
                    .as_deref()
                    .and_then(|head| resolve_blob_path(repo, head, &anchor.oid));

                if let Some(ref path) = path {
                    // Use the review's head commit as the commit_id for the GitHub API.
                    let commit_id = review_head.as_deref().unwrap_or(&anchor.oid);
                    match client
                        .create_review_comment(
                            &cfg.owner,
                            &cfg.repo,
                            github_number,
                            &comment.body,
                            commit_id,
                            path,
                            line,
                        )
                        .await
                    {
                        Ok(github_comment_id) => {
                            state.insert(
                                format!("comments/{github_comment_id}"),
                                comment.oid.clone(),
                            );
                            report.exported += 1;
                            continue;
                        }
                        Err(e) => {
                            eprintln!(
                                "forge: failed to export review comment {} on review {forge_review_oid}: {e:#}",
                                comment.oid
                            );
                            report.failed += 1;
                            continue;
                        }
                    }
                }
            }

            // Fall back to issue comment API for plain comments on PRs.
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
                        "forge: failed to export comment {} on review {forge_review_oid}: {e:#}",
                        comment.oid
                    );
                    report.failed += 1;
                }
            }
        }
    }

    Ok(report)
}

/// Export everything: pending issues + pending reviews + new comments for all synced entities.
///
/// # Errors
/// Returns an error if any export operation fails.
pub async fn export_all(
    repo: &Repository,
    cfg: &GitHubSyncConfig,
    client: &impl GitHubClient,
) -> Result<SyncReport> {
    let issue_report = export_issues(repo, cfg, client).await?;
    let review_report = export_reviews(repo, cfg, client).await?;
    Ok(SyncReport {
        imported: 0,
        exported: issue_report.exported + review_report.exported,
        skipped: issue_report.skipped + review_report.skipped,
        failed: issue_report.failed + review_report.failed,
        unexportable: issue_report.unexportable + review_report.unexportable,
    })
}
