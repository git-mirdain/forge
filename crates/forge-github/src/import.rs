//! GitHub → forge import functions.

use std::collections::HashMap;

use anyhow::{Context, Result};
use git_chain::Chain;
use git_forge::Store;
use git_forge::comment::{issue_comment_ref, review_comment_ref};
use git_forge::review::ReviewTarget;
use git_forge::sync::SyncReport;
use git2::Repository;

use crate::client::GitHubClient;
use crate::config::GitHubSyncConfig;
use crate::state::{load_sync_state, lookup_by_github_id, save_sync_state};

/// Import all GitHub issues from `cfg.owner`/`cfg.repo` into the local forge store.
///
/// Issues already recorded in the sync state are skipped. Each new issue stores
/// the GitHub issue URL as a `source` field for provenance.
///
/// # Errors
/// Returns an error if the GitHub API call fails or a git operation fails.
pub async fn import_issues(
    repo: &Repository,
    cfg: &GitHubSyncConfig,
    client: &impl GitHubClient,
) -> Result<SyncReport> {
    let mut state = load_sync_state(repo, &cfg.owner, &cfg.repo)?;
    let issues = client.fetch_issues(&cfg.owner, &cfg.repo).await?;

    let store = Store::new(repo);
    let mut report = SyncReport::default();

    for issue in &issues {
        let state_key = format!("issues/{}", issue.number);
        if state.contains_key(&state_key) {
            report.skipped += 1;
            continue;
        }

        let sigil = cfg.sigils.get("issue").map_or("GH#", String::as_str);
        let display_id = format!("{sigil}{}", issue.number);
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

    for issue in &issues {
        if state.contains_key(&format!("issues/{}", issue.number)) {
            let comment_report =
                import_issue_comments_with_state(repo, cfg, client, issue.number, &mut state)
                    .await?;
            report.imported += comment_report.imported;
            report.skipped += comment_report.skipped;
            report.failed += comment_report.failed;
        }
    }

    save_sync_state(repo, &cfg.owner, &cfg.repo, &state)?;
    Ok(report)
}

/// Import comments for a single GitHub issue into the local forge chain.
///
/// # Errors
/// Returns an error if the GitHub API call fails or a git operation fails.
pub async fn import_issue_comments(
    repo: &Repository,
    cfg: &GitHubSyncConfig,
    client: &impl GitHubClient,
    github_number: u64,
) -> Result<SyncReport> {
    let mut state = load_sync_state(repo, &cfg.owner, &cfg.repo)?;
    let report =
        import_issue_comments_with_state(repo, cfg, client, github_number, &mut state).await?;
    save_sync_state(repo, &cfg.owner, &cfg.repo, &state)?;
    Ok(report)
}

async fn import_issue_comments_with_state(
    repo: &Repository,
    cfg: &GitHubSyncConfig,
    client: &impl GitHubClient,
    github_number: u64,
    state: &mut HashMap<String, String>,
) -> Result<SyncReport> {
    let forge_issue_oid = match lookup_by_github_id(state, "issues", github_number) {
        Some(oid) => oid.to_string(),
        None => return Ok(SyncReport::default()),
    };

    let comments = client
        .fetch_issue_comments(&cfg.owner, &cfg.repo, github_number)
        .await?;
    let ref_name = issue_comment_ref(&forge_issue_oid);
    let mut report = SyncReport::default();

    for comment in &comments {
        let state_key = format!("comments/{}", comment.id);
        if state.contains_key(&state_key) {
            report.skipped += 1;
            continue;
        }

        let body = comment.body.as_deref().unwrap_or("");
        let message = if body.is_empty() {
            format!("Github-Id: {}", comment.id)
        } else {
            format!("{body}\n\nGithub-Id: {}", comment.id)
        };

        let tree = repo.build_tree(&[])?;
        match repo.append(&ref_name, &message, tree, None) {
            Ok(entry) => {
                state.insert(state_key, entry.commit.to_string());
                report.imported += 1;
            }
            Err(e) => {
                eprintln!(
                    "forge: failed to import comment {} on issue {github_number}: {e}",
                    comment.id
                );
                report.failed += 1;
            }
        }
    }

    Ok(report)
}

/// Import all GitHub pull requests from `cfg.owner`/`cfg.repo` into the local forge store.
///
/// # Errors
/// Returns an error if the GitHub API call fails or a git operation fails.
pub async fn import_reviews(
    repo: &Repository,
    cfg: &GitHubSyncConfig,
    client: &impl GitHubClient,
) -> Result<SyncReport> {
    let mut state = load_sync_state(repo, &cfg.owner, &cfg.repo)?;
    let pulls = client.fetch_pulls(&cfg.owner, &cfg.repo).await?;

    let store = Store::new(repo);
    let mut report = SyncReport::default();

    for pull in &pulls {
        let state_key = format!("reviews/{}", pull.number);
        if state.contains_key(&state_key) {
            report.skipped += 1;
            continue;
        }

        let sigil = cfg.sigils.get("review").map_or("GH#", String::as_str);
        let display_id = format!("{sigil}{}", pull.number);
        let login = &pull.user.login;
        let email = format!("{login}@users.noreply.github.com");
        let source = format!(
            "https://github.com/{}/{}/pull/{}",
            cfg.owner, cfg.repo, pull.number
        );

        let author = git2::Signature::now(login, &email)
            .with_context(|| format!("invalid git signature for PR {}", pull.number))?;

        let body = pull.body.as_deref().unwrap_or("");

        let review_state = if pull.merged || pull.state == "closed" {
            git_forge::review::ReviewState::Closed
        } else {
            git_forge::review::ReviewState::Open
        };

        let target = ReviewTarget {
            head: pull.head.sha.clone(),
            base: Some(pull.base.sha.clone()),
        };
        let source_ref = Some(pull.head.ref_field.as_str());

        match store.create_review_imported(
            &pull.title,
            body,
            &target,
            source_ref,
            Some(&review_state),
            &display_id,
            &author,
            &source,
        ) {
            Ok(created) => {
                state.insert(state_key, created.oid.clone());
                report.imported += 1;
            }
            Err(e) => {
                eprintln!("forge: failed to import PR {}: {e}", pull.number);
                report.failed += 1;
            }
        }
    }

    // Import review comments for each imported/existing review.
    for pull in &pulls {
        if state.contains_key(&format!("reviews/{}", pull.number)) {
            let comment_report =
                import_review_comments_with_state(repo, cfg, client, pull.number, &mut state)
                    .await?;
            report.imported += comment_report.imported;
            report.skipped += comment_report.skipped;
            report.failed += comment_report.failed;
        }
    }

    save_sync_state(repo, &cfg.owner, &cfg.repo, &state)?;
    Ok(report)
}

/// Import review comments for a single pull request.
///
/// # Errors
/// Returns an error if the GitHub API call fails or a git operation fails.
pub async fn import_review_comments(
    repo: &Repository,
    cfg: &GitHubSyncConfig,
    client: &impl GitHubClient,
    github_number: u64,
) -> Result<SyncReport> {
    let mut state = load_sync_state(repo, &cfg.owner, &cfg.repo)?;
    let report =
        import_review_comments_with_state(repo, cfg, client, github_number, &mut state).await?;
    save_sync_state(repo, &cfg.owner, &cfg.repo, &state)?;
    Ok(report)
}

async fn import_review_comments_with_state(
    repo: &Repository,
    cfg: &GitHubSyncConfig,
    client: &impl GitHubClient,
    github_number: u64,
    state: &mut HashMap<String, String>,
) -> Result<SyncReport> {
    let forge_review_oid = match lookup_by_github_id(state, "reviews", github_number) {
        Some(oid) => oid.to_string(),
        None => return Ok(SyncReport::default()),
    };

    let comments = client
        .fetch_review_comments(&cfg.owner, &cfg.repo, github_number)
        .await?;
    let ref_name = review_comment_ref(&forge_review_oid);
    let mut report = SyncReport::default();

    for comment in &comments {
        let state_key = format!("comments/{}", comment.id);
        if state.contains_key(&state_key) {
            report.skipped += 1;
            continue;
        }

        let body = comment.body.as_deref().unwrap_or("");

        // Build trailer block with anchor info and Github-Id.
        let mut trailer_lines = Vec::new();
        if let Some(ref path) = comment.path {
            trailer_lines.push(format!("Anchor: {}", comment.commit_id));
            trailer_lines.push(format!("Anchor-Path: {path}"));
            if let Some(l) = comment.line {
                trailer_lines.push(format!("Anchor-Range: {l}-{l}"));
            }
        }
        trailer_lines.push(format!("Github-Id: {}", comment.id));
        let trailers = trailer_lines.join("\n");

        let message = if body.is_empty() {
            trailers
        } else {
            format!("{body}\n\n{trailers}")
        };

        let tree = repo.build_tree(&[])?;
        match repo.append(&ref_name, &message, tree, None) {
            Ok(entry) => {
                state.insert(state_key, entry.commit.to_string());
                report.imported += 1;
            }
            Err(e) => {
                eprintln!(
                    "forge: failed to import review comment {} on PR {github_number}: {e}",
                    comment.id
                );
                report.failed += 1;
            }
        }
    }

    Ok(report)
}

/// Import everything: issues, reviews, and all their comments.
///
/// # Errors
/// Returns an error if any import operation fails.
pub async fn import_all(
    repo: &Repository,
    cfg: &GitHubSyncConfig,
    client: &impl GitHubClient,
) -> Result<SyncReport> {
    let issue_report = import_issues(repo, cfg, client).await?;
    let review_report = import_reviews(repo, cfg, client).await?;
    Ok(SyncReport {
        imported: issue_report.imported + review_report.imported,
        exported: 0,
        skipped: issue_report.skipped + review_report.skipped,
        failed: issue_report.failed + review_report.failed,
    })
}
