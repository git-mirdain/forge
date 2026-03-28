//! Thin `octocrab` wrapper for GitHub REST API operations.

use anyhow::{Context, Result, bail};
use octocrab::Octocrab;
use serde::Deserialize;

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

/// A GitHub issue as returned by the list/get API.
#[derive(Debug, Deserialize)]
pub struct GhIssue {
    /// GitHub issue number.
    pub number: u64,
    /// Issue title.
    pub title: String,
    /// Issue body (may be absent).
    pub body: Option<String>,
    /// `"open"` or `"closed"`.
    pub state: String,
    /// Labels attached to the issue.
    pub labels: Vec<GhLabel>,
    /// Assigned users.
    pub assignees: Vec<GhUser>,
    /// Issue author.
    pub user: GhUser,
    /// Creation timestamp (ISO 8601).
    pub created_at: String,
    /// Present when this entry is actually a pull request.
    pub pull_request: Option<serde_json::Value>,
}

/// A comment on a GitHub issue.
#[derive(Debug, Deserialize)]
pub struct GhIssueComment {
    /// GitHub comment ID.
    pub id: u64,
    /// Comment body (may be absent).
    pub body: Option<String>,
    /// Comment author.
    pub user: GhUser,
    /// Creation timestamp (ISO 8601).
    pub created_at: String,
}

/// A label name wrapper.
#[derive(Debug, Deserialize)]
pub struct GhLabel {
    /// Label name.
    pub name: String,
}

/// A GitHub user login wrapper.
#[derive(Debug, Deserialize)]
pub struct GhUser {
    /// GitHub login handle.
    pub login: String,
}

// ---------------------------------------------------------------------------
// Client constructor
// ---------------------------------------------------------------------------

/// Build an `Octocrab` client.
///
/// Uses `token` if provided; otherwise falls back to the `GH_TOKEN` env var.
///
/// # Errors
/// Returns an error if `token` is `None` and `GH_TOKEN` is not set, or if
/// the octocrab builder fails.
pub fn make_client(token: Option<&str>) -> Result<Octocrab> {
    let tok = match token {
        Some(t) => t.to_string(),
        None => std::env::var("GH_TOKEN").context("no token provided and GH_TOKEN is not set")?,
    };
    let client = Octocrab::builder()
        .personal_token(tok)
        .build()
        .context("failed to build octocrab client")?;
    Ok(client)
}

// ---------------------------------------------------------------------------
// Fetch
// ---------------------------------------------------------------------------

/// Fetch all non-PR issues from the repository, paging through all results.
///
/// # Errors
/// Returns an error if any GitHub API call fails or the response cannot be
/// deserialized.
pub async fn fetch_issues(client: &Octocrab, owner: &str, repo: &str) -> Result<Vec<GhIssue>> {
    let mut page: u32 = 1;
    let mut all = Vec::new();
    loop {
        let url =
            format!("/repos/{owner}/{repo}/issues?state=all&filter=all&per_page=100&page={page}");
        let items: Vec<GhIssue> = client.get(&url, None::<&()>).await?;
        let done = items.len() < 100;
        for issue in items {
            if issue.pull_request.is_none() {
                all.push(issue);
            }
        }
        if done {
            break;
        }
        page += 1;
    }
    Ok(all)
}

/// Fetch all comments for the given issue number, paging through all results.
///
/// # Errors
/// Returns an error if any GitHub API call fails or the response cannot be
/// deserialized.
pub async fn fetch_issue_comments(
    client: &Octocrab,
    owner: &str,
    repo: &str,
    number: u64,
) -> Result<Vec<GhIssueComment>> {
    let mut page: u32 = 1;
    let mut all = Vec::new();
    loop {
        let url =
            format!("/repos/{owner}/{repo}/issues/{number}/comments?per_page=100&page={page}");
        let items: Vec<GhIssueComment> = client.get(&url, None::<&()>).await?;
        let done = items.len() < 100;
        all.extend(items);
        if done {
            break;
        }
        page += 1;
    }
    Ok(all)
}

// ---------------------------------------------------------------------------
// Create / update
// ---------------------------------------------------------------------------

/// Create a GitHub issue and return its number.
///
/// # Errors
/// Returns an error if the API call fails or the response cannot be deserialized.
pub async fn create_github_issue(
    client: &Octocrab,
    owner: &str,
    repo: &str,
    title: &str,
    body: &str,
    labels: &[String],
    assignees: &[String],
) -> Result<u64> {
    #[derive(serde::Serialize)]
    struct Payload<'a> {
        title: &'a str,
        body: &'a str,
        labels: &'a [String],
        assignees: &'a [String],
    }
    let payload = Payload {
        title,
        body,
        labels,
        assignees,
    };
    let url = format!("/repos/{owner}/{repo}/issues");
    let issue: GhIssue = client.post(&url, Some(&payload)).await?;
    Ok(issue.number)
}

/// Update an existing GitHub issue.
///
/// All fields are optional; passing all `None` is an error.
///
/// # Errors
/// Returns an error if all fields are `None`, or if the API call fails.
#[allow(clippy::too_many_arguments)]
pub async fn update_github_issue(
    client: &Octocrab,
    owner: &str,
    repo: &str,
    number: u64,
    title: Option<&str>,
    body: Option<&str>,
    state: Option<&str>,
    labels: Option<&[String]>,
    assignees: Option<&[String]>,
) -> Result<()> {
    #[derive(serde::Serialize)]
    struct Payload<'a> {
        #[serde(skip_serializing_if = "Option::is_none")]
        title: Option<&'a str>,
        #[serde(skip_serializing_if = "Option::is_none")]
        body: Option<&'a str>,
        #[serde(skip_serializing_if = "Option::is_none")]
        state: Option<&'a str>,
        #[serde(skip_serializing_if = "Option::is_none")]
        labels: Option<&'a [String]>,
        #[serde(skip_serializing_if = "Option::is_none")]
        assignees: Option<&'a [String]>,
    }
    if title.is_none()
        && body.is_none()
        && state.is_none()
        && labels.is_none()
        && assignees.is_none()
    {
        bail!("update_github_issue called with no fields to update");
    }
    let payload = Payload {
        title,
        body,
        state,
        labels,
        assignees,
    };
    let url = format!("/repos/{owner}/{repo}/issues/{number}");
    let _: serde_json::Value = client.patch(&url, Some(&payload)).await?;
    Ok(())
}

/// Create a comment on a GitHub issue and return the comment ID.
///
/// # Errors
/// Returns an error if the API call fails or the response cannot be deserialized.
pub async fn create_github_issue_comment(
    client: &Octocrab,
    owner: &str,
    repo: &str,
    number: u64,
    body: &str,
) -> Result<u64> {
    #[derive(serde::Serialize)]
    struct Payload<'a> {
        body: &'a str,
    }
    let payload = Payload { body };
    let url = format!("/repos/{owner}/{repo}/issues/{number}/comments");
    let comment: GhIssueComment = client.post(&url, Some(&payload)).await?;
    Ok(comment.id)
}
