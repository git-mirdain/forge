//! Integration tests for GitHub issue import/export.
#![allow(clippy::must_use_candidate, clippy::missing_panics_doc, missing_docs)]

use std::cell::Cell;
use std::collections::BTreeMap;

use anyhow::Result;
use forge_github::client::{GhIssue, GhIssueComment, GhLabel, GhUser, GitHubClient};
use forge_github::config::GitHubSyncConfig;
use git2::Repository;
use tempfile::TempDir;

// ---------------------------------------------------------------------------
// Mock client
// ---------------------------------------------------------------------------

/// A [`GitHubClient`] that returns canned data and records write calls.
pub struct MockClient {
    /// Issues returned by [`GitHubClient::fetch_issues`].
    pub issues: Vec<GhIssue>,
    /// The next number returned by [`GitHubClient::create_issue`].
    pub next_number: Cell<u64>,
}

impl MockClient {
    pub fn new(issues: Vec<GhIssue>) -> Self {
        Self {
            issues,
            next_number: Cell::new(1),
        }
    }

    pub fn empty() -> Self {
        Self::new(Vec::new())
    }
}

impl GitHubClient for MockClient {
    async fn fetch_issues(&self, _owner: &str, _repo: &str) -> Result<Vec<GhIssue>> {
        Ok(self.issues.clone())
    }

    async fn fetch_issue_comments(
        &self,
        _owner: &str,
        _repo: &str,
        _number: u64,
    ) -> Result<Vec<GhIssueComment>> {
        Ok(Vec::new())
    }

    async fn create_issue(
        &self,
        _owner: &str,
        _repo: &str,
        _title: &str,
        _body: &str,
        _labels: &[String],
        _assignees: &[String],
    ) -> Result<u64> {
        let n = self.next_number.get();
        self.next_number.set(n + 1);
        Ok(n)
    }

    async fn update_issue(
        &self,
        _owner: &str,
        _repo: &str,
        _number: u64,
        _title: Option<&str>,
        _body: Option<&str>,
        _state: Option<&str>,
        _labels: Option<&[String]>,
        _assignees: Option<&[String]>,
    ) -> Result<()> {
        Ok(())
    }

    async fn create_issue_comment(
        &self,
        _owner: &str,
        _repo: &str,
        _number: u64,
        _body: &str,
    ) -> Result<u64> {
        Ok(0)
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Create a temporary git repository with an initial empty commit.
pub fn test_repo() -> (TempDir, Repository) {
    let dir = TempDir::new().expect("failed to create temp dir");
    let repo = Repository::init(dir.path()).expect("failed to init repo");
    {
        let mut index = repo.index().expect("failed to get index");
        let tree_oid = index.write_tree().expect("failed to write tree");
        let tree = repo.find_tree(tree_oid).expect("failed to find tree");
        let sig = repo.signature().unwrap_or_else(|_| {
            git2::Signature::now("test", "test@test.com").expect("failed to create signature")
        });
        repo.commit(Some("HEAD"), &sig, &sig, "initial", &tree, &[])
            .expect("failed to create initial commit");
    }
    (dir, repo)
}

/// Build a [`GitHubSyncConfig`] suitable for tests.
pub fn test_config() -> GitHubSyncConfig {
    GitHubSyncConfig {
        owner: "test-owner".into(),
        repo: "test-repo".into(),
        sigils: BTreeMap::new(),
        token: None,
    }
}

/// Build a [`GhIssue`] with sensible defaults.
pub fn gh_issue(number: u64, title: &str) -> GhIssue {
    GhIssue {
        number,
        title: title.to_string(),
        body: Some(format!("body of {title}")),
        state: "open".into(),
        labels: vec![GhLabel { name: "bug".into() }],
        assignees: Vec::new(),
        user: GhUser {
            login: "octocat".into(),
        },
        created_at: "2025-01-01T00:00:00Z".into(),
        pull_request: None,
    }
}
