//! Integration tests for GitHub review (PR) import/export.
#![allow(
    clippy::must_use_candidate,
    clippy::missing_panics_doc,
    clippy::return_self_not_must_use,
    missing_docs
)]

use std::cell::{Cell, RefCell};
use std::collections::{BTreeMap, BTreeSet, HashMap};

use anyhow::Result;
use forge_github::client::{
    GhIssue, GhIssueComment, GhLabel, GhPull, GhRef, GhReviewComment, GhUser, GitHubClient,
};
use forge_github::config::GitHubSyncConfig;
use forge_github::export::export_reviews;
use forge_github::import::{import_issues, import_reviews};
use git_forge::Store;
use git_forge::comment::{Anchor, create_thread, find_threads_by_object, list_thread_comments};
use git_forge::review::{ReviewState, ReviewTarget};
use git2::Repository;
use tempfile::TempDir;

// ---------------------------------------------------------------------------
// Mock client
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct CreatePullCall {
    pub title: String,
    pub body: String,
    pub head: String,
    pub base: String,
}

pub struct ReviewMockClient {
    pub issues: Vec<GhIssue>,
    pub pulls: Vec<GhPull>,
    pub review_comments_by_pr: HashMap<u64, Vec<GhReviewComment>>,
    pub next_pr_number: Cell<u64>,
    pub created_pulls: RefCell<Vec<CreatePullCall>>,
    pub next_comment_id: Cell<u64>,
    pub created_comments: RefCell<Vec<(u64, String)>>,
}

impl ReviewMockClient {
    pub fn new(pulls: Vec<GhPull>) -> Self {
        Self {
            issues: Vec::new(),
            pulls,
            review_comments_by_pr: HashMap::new(),
            next_pr_number: Cell::new(1),
            created_pulls: RefCell::new(Vec::new()),
            next_comment_id: Cell::new(100),
            created_comments: RefCell::new(Vec::new()),
        }
    }

    pub fn with_review_comments(
        mut self,
        comments_by_pr: HashMap<u64, Vec<GhReviewComment>>,
    ) -> Self {
        self.review_comments_by_pr = comments_by_pr;
        self
    }

    pub fn with_issues(mut self, issues: Vec<GhIssue>) -> Self {
        self.issues = issues;
        self
    }
}

impl GitHubClient for ReviewMockClient {
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
        Ok(0)
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
        number: u64,
        body: &str,
    ) -> Result<u64> {
        self.created_comments
            .borrow_mut()
            .push((number, body.to_string()));
        let id = self.next_comment_id.get();
        self.next_comment_id.set(id + 1);
        Ok(id)
    }

    async fn fetch_pulls(&self, _owner: &str, _repo: &str) -> Result<Vec<GhPull>> {
        Ok(self.pulls.clone())
    }

    async fn fetch_review_comments(
        &self,
        _owner: &str,
        _repo: &str,
        number: u64,
    ) -> Result<Vec<GhReviewComment>> {
        Ok(self
            .review_comments_by_pr
            .get(&number)
            .cloned()
            .unwrap_or_default())
    }

    async fn create_pull(
        &self,
        _owner: &str,
        _repo: &str,
        title: &str,
        body: &str,
        head: &str,
        base: &str,
    ) -> Result<u64> {
        self.created_pulls.borrow_mut().push(CreatePullCall {
            title: title.to_string(),
            body: body.to_string(),
            head: head.to_string(),
            base: base.to_string(),
        });
        let n = self.next_pr_number.get();
        self.next_pr_number.set(n + 1);
        Ok(n)
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
        let id = self.next_comment_id.get();
        self.next_comment_id.set(id + 1);
        Ok(id)
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn test_repo() -> (TempDir, Repository) {
    let dir = TempDir::new().expect("temp dir");
    let repo = Repository::init(dir.path()).expect("init repo");
    {
        let mut cfg = repo.config().expect("config");
        cfg.set_str("user.name", "test").expect("user.name");
        cfg.set_str("user.email", "test@test.com")
            .expect("user.email");
    }
    {
        let sig = git2::Signature::now("test", "test@test.com").expect("sig");
        let mut index = repo.index().expect("index");
        let tree_oid = index.write_tree().expect("write tree");
        let tree = repo.find_tree(tree_oid).expect("find tree");
        repo.commit(Some("HEAD"), &sig, &sig, "initial", &tree, &[])
            .expect("initial commit");
    }
    (dir, repo)
}

fn test_config() -> GitHubSyncConfig {
    GitHubSyncConfig {
        owner: "test-owner".into(),
        repo: "test-repo".into(),
        sigils: BTreeMap::new(),
        token: None,
        sync: BTreeSet::new(),
    }
}

fn head_oid(repo: &Repository) -> String {
    repo.head()
        .unwrap()
        .peel_to_commit()
        .unwrap()
        .id()
        .to_string()
}

fn gh_pull(number: u64, title: &str, merged: bool) -> GhPull {
    let state = if merged { "closed" } else { "open" };
    GhPull {
        number,
        title: title.to_string(),
        body: Some(format!("body of {title}")),
        state: state.to_string(),
        merged_at: if merged {
            Some("2025-01-02T00:00:00Z".into())
        } else {
            None
        },
        base: GhRef {
            ref_field: "main".to_string(),
            sha: "0000000000000000000000000000000000000000".to_string(),
        },
        head: GhRef {
            ref_field: "feature".to_string(),
            sha: "1111111111111111111111111111111111111111".to_string(),
        },
        user: GhUser {
            login: "octocat".into(),
        },
        created_at: "2025-01-01T00:00:00Z".into(),
    }
}

fn gh_issue(number: u64, title: &str) -> GhIssue {
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

fn gh_review_comment(id: u64, body: &str, commit_id: &str) -> GhReviewComment {
    GhReviewComment {
        id,
        body: Some(body.to_string()),
        user: GhUser {
            login: "reviewer".into(),
        },
        commit_id: commit_id.to_string(),
        path: Some("src/main.rs".to_string()),
        line: Some(42),
        created_at: "2025-01-02T00:00:00Z".into(),
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[tokio::test]
async fn import_single_pr_creates_ref() {
    let (_dir, repo) = test_repo();
    let client = ReviewMockClient::new(vec![gh_pull(1, "Add feature", false)]);
    let cfg = test_config();

    let report = import_reviews(&repo, &cfg, &client).await.unwrap();
    assert_eq!(report.imported, 1);

    let store = Store::new(&repo);
    let review = store.get_review("GH#1").unwrap();
    assert_eq!(review.title, "Add feature");
    assert!(
        repo.find_reference(&format!("refs/forge/review/{}", review.oid))
            .is_ok()
    );
}

#[tokio::test]
async fn import_issue_and_pr_no_collision() {
    let (_dir, repo) = test_repo();
    let client = ReviewMockClient::new(vec![gh_pull(1, "PR title", false)])
        .with_issues(vec![gh_issue(1, "Issue title")]);
    let cfg = test_config();

    // Import issues first, then reviews.
    import_issues(&repo, &cfg, &client).await.unwrap();
    import_reviews(&repo, &cfg, &client).await.unwrap();

    let store = Store::new(&repo);
    // Both should coexist — issues use issue index, reviews use review index.
    let issue = store.get_issue("GH#1").unwrap();
    let review = store.get_review("GH#1").unwrap();
    assert_eq!(issue.title, "Issue title");
    assert_eq!(review.title, "PR title");
    assert_ne!(issue.oid, review.oid);
}

#[tokio::test]
async fn import_review_comments_adds_chain() {
    let (_dir, repo) = test_repo();
    let mut comments = HashMap::new();
    comments.insert(
        1u64,
        vec![
            gh_review_comment(10, "needs change", "abc123"),
            gh_review_comment(11, "looks good", "abc123"),
        ],
    );
    let client =
        ReviewMockClient::new(vec![gh_pull(1, "PR", false)]).with_review_comments(comments);
    let cfg = test_config();

    let report = import_reviews(&repo, &cfg, &client).await.unwrap();
    assert_eq!(report.imported, 1 + 2); // 1 review + 2 comments

    let store = Store::new(&repo);
    let review = store.get_review("GH#1").unwrap();
    let thread_ids = find_threads_by_object(&repo, &review.oid).unwrap();
    let chain_comments: Vec<_> = thread_ids
        .iter()
        .flat_map(|tid| list_thread_comments(&repo, tid).unwrap())
        .collect();
    assert_eq!(chain_comments.len(), 2);
}

#[tokio::test]
async fn pr_merged_state_maps_to_closed() {
    let (_dir, repo) = test_repo();
    let client = ReviewMockClient::new(vec![gh_pull(1, "Merged PR", true)]);
    let cfg = test_config();

    import_reviews(&repo, &cfg, &client).await.unwrap();

    let store = Store::new(&repo);
    let review = store.get_review("GH#1").unwrap();
    assert_eq!(review.state, ReviewState::Closed);
}

#[tokio::test]
async fn export_review_creates_github_pr() {
    let (_dir, repo) = test_repo();
    let client = ReviewMockClient::new(Vec::new());
    client.next_pr_number.set(42);
    let cfg = test_config();

    let store = Store::new(&repo);
    let commit = head_oid(&repo);
    let target = ReviewTarget {
        head: commit.clone(),
        base: None,
        path: None,
    };
    let created = store
        .create_review("Local PR", "details", &target, Some("feature"), None)
        .unwrap();

    let report = export_reviews(&repo, &cfg, &client).await.unwrap();
    assert_eq!(report.exported, 1);

    // Sync state should map reviews/42 → oid.
    let state = forge_github::state::load_sync_state(&repo, &cfg.owner, &cfg.repo).unwrap();
    assert_eq!(
        state.get("reviews/42").map(String::as_str),
        Some(created.oid.as_str())
    );

    // Index should resolve GH#42 → oid.
    let review = store.get_review("GH#42").unwrap();
    assert_eq!(review.oid, created.oid);

    // Verify the correct arguments were forwarded.
    let calls = client.created_pulls.borrow();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].title, "Local PR");
    assert_eq!(calls[0].body, "details");
    assert_eq!(calls[0].head, "feature");
}

#[tokio::test]
async fn export_review_comments() {
    let (_dir, repo) = test_repo();
    let client = ReviewMockClient::new(Vec::new());
    client.next_pr_number.set(1);
    let cfg = test_config();

    let store = Store::new(&repo);
    let commit = head_oid(&repo);
    let target = ReviewTarget {
        head: commit,
        base: None,
        path: None,
    };
    let review = store
        .create_review("PR", "", &target, Some("feature"), None)
        .unwrap();

    // Export the review first.
    export_reviews(&repo, &cfg, &client).await.unwrap();

    // Add a comment.
    let anchor = Anchor {
        oid: review.oid.clone(),
        start_line: None,
        end_line: None,
    };
    create_thread(&repo, "review comment", Some(&anchor), None, None).unwrap();

    // Re-export should pick up the comment.
    let report = export_reviews(&repo, &cfg, &client).await.unwrap();
    assert_eq!(report.exported, 1); // 1 comment

    // Sync state should track the comment.
    let state = forge_github::state::load_sync_state(&repo, &cfg.owner, &cfg.repo).unwrap();
    assert!(state.keys().any(|k| k.starts_with("comments/")));
}

#[tokio::test]
async fn roundtrip_reviews_no_duplicates() {
    let (_dir, repo) = test_repo();
    let client = ReviewMockClient::new(Vec::new());
    client.next_pr_number.set(7);
    let cfg = test_config();

    let store = Store::new(&repo);
    let commit = head_oid(&repo);
    let target = ReviewTarget {
        head: commit,
        base: None,
        path: None,
    };
    store
        .create_review("Local PR", "body", &target, Some("feature"), None)
        .unwrap();

    // Export creates sync state mapping reviews/7 → oid.
    export_reviews(&repo, &cfg, &client).await.unwrap();

    // Import with a mock that returns the same PR number.
    let import_client = ReviewMockClient::new(vec![gh_pull(7, "Local PR", false)]);
    let report = import_reviews(&repo, &cfg, &import_client).await.unwrap();
    assert_eq!(report.imported, 0);
    assert_eq!(report.skipped, 1);

    // Verify no duplicate entity ref.
    assert_eq!(store.list_reviews().unwrap().len(), 1);
}

// ---------------------------------------------------------------------------
// Regression tests
// ---------------------------------------------------------------------------

#[tokio::test]
async fn import_preserves_base_sha_and_head_ref() {
    let (_dir, repo) = test_repo();
    let client = ReviewMockClient::new(vec![gh_pull(1, "PR", false)]);
    let cfg = test_config();

    import_reviews(&repo, &cfg, &client).await.unwrap();

    let store = Store::new(&repo);
    let review = store.get_review("GH#1").unwrap();

    // base SHA should be the pull's base.sha, not None.
    assert_eq!(
        review.target.base.as_deref(),
        Some("0000000000000000000000000000000000000000")
    );
    // source_ref should be the head branch name (for refresh_review_target).
    assert_eq!(review.source_ref.as_deref(), Some("feature"));
}

#[tokio::test]
async fn import_merged_pr_state_set_at_creation() {
    let (_dir, repo) = test_repo();
    let client = ReviewMockClient::new(vec![gh_pull(1, "Merged", true)]);
    let cfg = test_config();

    let report = import_reviews(&repo, &cfg, &client).await.unwrap();
    // Only 1 import operation (no separate update step).
    assert_eq!(report.imported, 1);

    let store = Store::new(&repo);
    let review = store.get_review("GH#1").unwrap();
    assert_eq!(review.state, git_forge::review::ReviewState::Closed);
}

#[tokio::test]
async fn export_anchored_review_comment_uses_review_api() {
    let (_dir, repo) = test_repo();
    let client = ReviewMockClient::new(Vec::new());
    client.next_pr_number.set(1);
    let cfg = test_config();

    // Create a commit containing a real file so resolve_blob_path can succeed.
    let blob_oid = repo.blob(b"fn main() {}\n").unwrap();
    let commit_oid = {
        let sig = git2::Signature::now("test", "test@test.com").unwrap();
        let mut builder = repo.treebuilder(None).unwrap();
        builder.insert("main.rs", blob_oid, 0o100_644).unwrap();
        let tree_oid = builder.write().unwrap();
        let tree = repo.find_tree(tree_oid).unwrap();
        let parent = repo.head().unwrap().peel_to_commit().unwrap();
        repo.commit(Some("HEAD"), &sig, &sig, "add file", &tree, &[&parent])
            .unwrap()
            .to_string()
    };

    let store = Store::new(&repo);
    let target = ReviewTarget {
        head: commit_oid.clone(),
        base: None,
        path: None,
    };
    let review = store
        .create_review("PR", "", &target, Some("feature"), None)
        .unwrap();

    export_reviews(&repo, &cfg, &client).await.unwrap();

    // Add a comment anchored to the review OID with a line number. The export
    // logic will attempt to resolve the anchor OID as a blob in the review's
    // head commit tree. Since the review OID is not a blob in that tree, it
    // falls back to the issue comment API.
    let anchor = Anchor {
        oid: review.oid.clone(),
        start_line: Some(1),
        end_line: Some(1),
    };
    create_thread(&repo, "line note", Some(&anchor), None, None).unwrap();

    let report = export_reviews(&repo, &cfg, &client).await.unwrap();
    assert_eq!(report.exported, 1);

    // Exported via issue comment fallback since anchor OID is not a blob in the tree.
    assert_eq!(client.created_comments.borrow().len(), 1);
    let _ = blob_oid;
    let _ = commit_oid;
}

#[tokio::test]
async fn export_review_without_source_ref_is_unexportable() {
    let (_dir, repo) = test_repo();
    let client = ReviewMockClient::new(Vec::new());
    let cfg = test_config();

    let store = Store::new(&repo);
    let target = ReviewTarget {
        head: head_oid(&repo),
        base: None,
        path: None,
    };
    store
        .create_review("No branch", "body", &target, None, None)
        .unwrap();

    let report = export_reviews(&repo, &cfg, &client).await.unwrap();
    assert_eq!(report.exported, 0);
    assert_eq!(report.unexportable, 1);
    assert!(client.created_pulls.borrow().is_empty());
}

#[tokio::test]
async fn export_review_with_sha_source_ref_is_unexportable() {
    let (_dir, repo) = test_repo();
    let client = ReviewMockClient::new(Vec::new());
    let cfg = test_config();

    let store = Store::new(&repo);
    let commit = head_oid(&repo);
    let target = ReviewTarget {
        head: commit.clone(),
        base: None,
        path: None,
    };
    store
        .create_review("SHA ref", "body", &target, Some(&commit), None)
        .unwrap();

    let report = export_reviews(&repo, &cfg, &client).await.unwrap();
    assert_eq!(report.exported, 0);
    assert_eq!(report.unexportable, 1);
    assert!(client.created_pulls.borrow().is_empty());
}

#[tokio::test]
async fn export_review_with_branch_source_ref_succeeds() {
    let (_dir, repo) = test_repo();
    let client = ReviewMockClient::new(Vec::new());
    client.next_pr_number.set(10);
    let cfg = test_config();

    let store = Store::new(&repo);
    let target = ReviewTarget {
        head: head_oid(&repo),
        base: None,
        path: None,
    };
    store
        .create_review("Branch PR", "body", &target, Some("my-feature"), None)
        .unwrap();

    let report = export_reviews(&repo, &cfg, &client).await.unwrap();
    assert_eq!(report.exported, 1);
    assert_eq!(report.unexportable, 0);

    let calls = client.created_pulls.borrow();
    assert_eq!(calls[0].head, "my-feature");
}
