//! Integration tests for GitHub issue import/export.
#![allow(clippy::must_use_candidate, clippy::missing_panics_doc, missing_docs)]

use std::cell::{Cell, RefCell};
use std::collections::BTreeMap;

use anyhow::Result;
use forge_github::client::{GhIssue, GhIssueComment, GhLabel, GhUser, GitHubClient};
use forge_github::config::GitHubSyncConfig;
use forge_github::export::export_issues;
use forge_github::import::import_issues;
use git_forge::Store;
use git2::Repository;
use tempfile::TempDir;

// ---------------------------------------------------------------------------
// Mock client
// ---------------------------------------------------------------------------

/// Arguments captured from a [`GitHubClient::create_issue`] call.
#[derive(Debug, Clone)]
pub struct CreateIssueCall {
    pub title: String,
    pub body: String,
    pub labels: Vec<String>,
    pub assignees: Vec<String>,
}

/// A [`GitHubClient`] that returns canned data and records write calls.
pub struct MockClient {
    /// Issues returned by [`GitHubClient::fetch_issues`].
    pub issues: Vec<GhIssue>,
    /// The next number returned by [`GitHubClient::create_issue`].
    pub next_number: Cell<u64>,
    /// Recorded [`GitHubClient::create_issue`] calls.
    pub created_issues: RefCell<Vec<CreateIssueCall>>,
}

impl MockClient {
    pub fn new(issues: Vec<GhIssue>) -> Self {
        Self {
            issues,
            next_number: Cell::new(1),
            created_issues: RefCell::new(Vec::new()),
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
        title: &str,
        body: &str,
        labels: &[String],
        assignees: &[String],
    ) -> Result<u64> {
        self.created_issues.borrow_mut().push(CreateIssueCall {
            title: title.to_string(),
            body: body.to_string(),
            labels: labels.to_vec(),
            assignees: assignees.to_vec(),
        });
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

    async fn fetch_pulls(
        &self,
        _owner: &str,
        _repo: &str,
    ) -> Result<Vec<forge_github::client::GhPull>> {
        Ok(Vec::new())
    }

    async fn fetch_review_comments(
        &self,
        _owner: &str,
        _repo: &str,
        _number: u64,
    ) -> Result<Vec<forge_github::client::GhReviewComment>> {
        Ok(Vec::new())
    }

    async fn create_pull(
        &self,
        _owner: &str,
        _repo: &str,
        _title: &str,
        _body: &str,
        _head: &str,
        _base: &str,
    ) -> Result<u64> {
        Ok(0)
    }

    async fn update_pull(
        &self,
        _owner: &str,
        _repo: &str,
        _number: u64,
        _title: Option<&str>,
        _body: Option<&str>,
        _state: Option<&str>,
    ) -> Result<()> {
        Ok(())
    }

    async fn create_review_comment(
        &self,
        _owner: &str,
        _repo: &str,
        _number: u64,
        _body: &str,
        _commit_id: &str,
        _path: &str,
        _line: u32,
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
        sync: vec![],
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

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[tokio::test]
async fn import_single_issue_creates_ref() {
    let (_dir, repo) = test_repo();
    let client = MockClient::new(vec![gh_issue(1, "Bug report")]);
    let cfg = test_config();

    let report = import_issues(&repo, &cfg, &client).await.unwrap();
    assert_eq!(report.imported, 1);

    // The issue ref should exist and be resolvable via display ID.
    let store = Store::new(&repo);
    let issue = store.get_issue("GH#1").unwrap();
    assert_eq!(issue.title, "Bug report");
    assert!(
        repo.find_reference(&format!("refs/forge/issue/{}", issue.oid))
            .is_ok()
    );
}

#[tokio::test]
async fn import_skips_already_imported() {
    let (_dir, repo) = test_repo();
    let client = MockClient::new(vec![gh_issue(1, "Bug report")]);
    let cfg = test_config();

    let r1 = import_issues(&repo, &cfg, &client).await.unwrap();
    assert_eq!(r1.imported, 1);
    assert_eq!(r1.skipped, 0);

    let r2 = import_issues(&repo, &cfg, &client).await.unwrap();
    assert_eq!(r2.imported, 0);
    assert_eq!(r2.skipped, 1);
}

#[tokio::test]
async fn sigil_configurable() {
    let (_dir, repo) = test_repo();
    let client = MockClient::new(vec![gh_issue(1, "Bug report")]);
    let mut cfg = test_config();
    cfg.sigils.insert("issue".into(), "ACME".into());

    import_issues(&repo, &cfg, &client).await.unwrap();

    let store = Store::new(&repo);
    let issue = store.get_issue("ACME1").unwrap();
    assert_eq!(issue.title, "Bug report");
    assert!(store.get_issue("GH#1").is_err());
}

#[tokio::test]
async fn export_issue_creates_github_issue() {
    let (_dir, repo) = test_repo();
    let client = MockClient::new(Vec::new());
    client.next_number.set(42);
    let cfg = test_config();

    let store = Store::new(&repo);
    let created = store
        .create_issue("Local bug", "details", &["bug"], &[])
        .unwrap();

    let report = export_issues(&repo, &cfg, &client).await.unwrap();
    assert_eq!(report.exported, 1);

    // Sync state should map issues/42 → oid.
    let state = forge_github::state::load_sync_state(&repo, &cfg.owner, &cfg.repo).unwrap();
    assert_eq!(
        state.get("issues/42").map(String::as_str),
        Some(created.oid.as_str())
    );

    // Index should resolve GH#42 → oid.
    let issue = store.get_issue("GH#42").unwrap();
    assert_eq!(issue.oid, created.oid);

    // Verify the correct arguments were forwarded to the API.
    let calls = client.created_issues.borrow();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].title, "Local bug");
    assert_eq!(calls[0].body, "details");
    assert_eq!(calls[0].labels, vec!["bug".to_string()]);
    assert!(calls[0].assignees.is_empty());
}

#[tokio::test]
async fn export_skips_already_exported() {
    let (_dir, repo) = test_repo();
    let client = MockClient::new(Vec::new());
    let cfg = test_config();

    let store = Store::new(&repo);
    store
        .create_issue("Local bug", "details", &["bug"], &[])
        .unwrap();

    let r1 = export_issues(&repo, &cfg, &client).await.unwrap();
    assert_eq!(r1.exported, 1);
    assert_eq!(r1.skipped, 0);

    let r2 = export_issues(&repo, &cfg, &client).await.unwrap();
    assert_eq!(r2.exported, 0);
    assert_eq!(r2.skipped, 1);
}

#[tokio::test]
async fn roundtrip_no_duplicates() {
    let (_dir, repo) = test_repo();
    let client = MockClient::new(Vec::new());
    client.next_number.set(7);
    let cfg = test_config();

    let store = Store::new(&repo);
    store
        .create_issue("Local bug", "details", &[], &[])
        .unwrap();

    // Export creates sync state mapping issues/7 → oid.
    export_issues(&repo, &cfg, &client).await.unwrap();

    // Now import with a mock that returns the same issue number.
    let import_client = MockClient::new(vec![gh_issue(7, "Local bug")]);
    let report = import_issues(&repo, &cfg, &import_client).await.unwrap();
    assert_eq!(report.imported, 0);
    assert_eq!(report.skipped, 1);

    // Verify no duplicate entity ref was created.
    assert_eq!(store.list_issues().unwrap().len(), 1);
}
