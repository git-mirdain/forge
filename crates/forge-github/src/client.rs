//! Thin `octocrab` wrapper for GitHub REST API operations.

use anyhow::{Context, Result, bail};
use octocrab::Octocrab;
use serde::Deserialize;

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

/// A GitHub issue as returned by the list/get API.
#[derive(Debug, Clone, Deserialize)]
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
#[derive(Debug, Clone, Deserialize)]
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
#[derive(Debug, Clone, Deserialize)]
pub struct GhLabel {
    /// Label name.
    pub name: String,
}

/// A GitHub user login wrapper.
#[derive(Debug, Clone, Deserialize)]
pub struct GhUser {
    /// GitHub login handle.
    pub login: String,
}

// ---------------------------------------------------------------------------
// Trait
// ---------------------------------------------------------------------------

/// Abstract interface to the GitHub REST API.
///
/// The production implementation ([`OctocrabClient`]) wraps `octocrab`.
/// Tests can supply a mock implementation with canned responses.
#[allow(clippy::too_many_arguments)]
pub trait GitHubClient {
    /// Fetch all non-PR issues from the repository.
    fn fetch_issues(
        &self,
        owner: &str,
        repo: &str,
    ) -> impl std::future::Future<Output = Result<Vec<GhIssue>>>;

    /// Fetch all comments for the given issue number.
    fn fetch_issue_comments(
        &self,
        owner: &str,
        repo: &str,
        number: u64,
    ) -> impl std::future::Future<Output = Result<Vec<GhIssueComment>>>;

    /// Create a GitHub issue and return its number.
    fn create_issue(
        &self,
        owner: &str,
        repo: &str,
        title: &str,
        body: &str,
        labels: &[String],
        assignees: &[String],
    ) -> impl std::future::Future<Output = Result<u64>>;

    /// Update an existing GitHub issue.
    fn update_issue(
        &self,
        owner: &str,
        repo: &str,
        number: u64,
        title: Option<&str>,
        body: Option<&str>,
        state: Option<&str>,
        labels: Option<&[String]>,
        assignees: Option<&[String]>,
    ) -> impl std::future::Future<Output = Result<()>>;

    /// Create a comment on a GitHub issue and return the comment ID.
    fn create_issue_comment(
        &self,
        owner: &str,
        repo: &str,
        number: u64,
        body: &str,
    ) -> impl std::future::Future<Output = Result<u64>>;
}

// ---------------------------------------------------------------------------
// Production implementation
// ---------------------------------------------------------------------------

/// Production [`GitHubClient`] backed by `octocrab`.
pub struct OctocrabClient {
    inner: Octocrab,
}

impl OctocrabClient {
    /// Build an `OctocrabClient`.
    ///
    /// Uses `token` if provided; otherwise falls back to the `GH_TOKEN` env var.
    ///
    /// # Errors
    /// Returns an error if `token` is `None` and `GH_TOKEN` is not set, or if
    /// the octocrab builder fails.
    pub fn new(token: Option<&str>) -> Result<Self> {
        let tok = match token {
            Some(t) => t.to_string(),
            None => {
                std::env::var("GH_TOKEN").context("no token provided and GH_TOKEN is not set")?
            }
        };
        let inner = Octocrab::builder()
            .personal_token(tok)
            .build()
            .context("failed to build octocrab client")?;
        Ok(Self { inner })
    }
}

impl GitHubClient for OctocrabClient {
    async fn fetch_issues(&self, owner: &str, repo: &str) -> Result<Vec<GhIssue>> {
        let mut page: u32 = 1;
        let mut all = Vec::new();
        loop {
            let url = format!(
                "/repos/{owner}/{repo}/issues?state=all&filter=all&per_page=100&page={page}"
            );
            let items: Vec<GhIssue> = self.inner.get(&url, None::<&()>).await?;
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

    async fn fetch_issue_comments(
        &self,
        owner: &str,
        repo: &str,
        number: u64,
    ) -> Result<Vec<GhIssueComment>> {
        let mut page: u32 = 1;
        let mut all = Vec::new();
        loop {
            let url =
                format!("/repos/{owner}/{repo}/issues/{number}/comments?per_page=100&page={page}");
            let items: Vec<GhIssueComment> = self.inner.get(&url, None::<&()>).await?;
            let done = items.len() < 100;
            all.extend(items);
            if done {
                break;
            }
            page += 1;
        }
        Ok(all)
    }

    async fn create_issue(
        &self,
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
        let issue: GhIssue = self.inner.post(&url, Some(&payload)).await?;
        Ok(issue.number)
    }

    #[allow(clippy::too_many_arguments)]
    async fn update_issue(
        &self,
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
            bail!("update_issue called with no fields to update");
        }
        let payload = Payload {
            title,
            body,
            state,
            labels,
            assignees,
        };
        let url = format!("/repos/{owner}/{repo}/issues/{number}");
        let _: serde_json::Value = self.inner.patch(&url, Some(&payload)).await?;
        Ok(())
    }

    async fn create_issue_comment(
        &self,
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
        let comment: GhIssueComment = self.inner.post(&url, Some(&payload)).await?;
        Ok(comment.id)
    }
}
