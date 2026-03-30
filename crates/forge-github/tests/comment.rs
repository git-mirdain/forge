//! Integration tests for GitHub issue comment import/export.
#![allow(clippy::must_use_candidate, clippy::missing_panics_doc, missing_docs)]

use std::cell::{Cell, RefCell};
use std::collections::{BTreeMap, HashMap};

use anyhow::Result;
use forge_github::client::{GhIssue, GhIssueComment, GhLabel, GhUser, GitHubClient};
use forge_github::config::GitHubSyncConfig;
use forge_github::export::{export_issue_comments, export_issues};
use forge_github::import::{import_issue_comments, import_issues};
use git_forge::Store;
use git_forge::comment::{add_comment, issue_comment_ref, list_comments};
use git2::Repository;
use tempfile::TempDir;

// ---------------------------------------------------------------------------
// Mock
// ---------------------------------------------------------------------------

pub struct CommentMockClient {
    pub issues: Vec<GhIssue>,
    pub comments_by_issue: HashMap<u64, Vec<GhIssueComment>>,
    pub next_comment_id: Cell<u64>,
    pub created_comments: RefCell<Vec<(u64, String)>>,
}

impl CommentMockClient {
    pub fn new(issues: Vec<GhIssue>, comments_by_issue: HashMap<u64, Vec<GhIssueComment>>) -> Self {
        Self {
            issues,
            comments_by_issue,
            next_comment_id: Cell::new(100),
            created_comments: RefCell::new(Vec::new()),
        }
    }
}

impl GitHubClient for CommentMockClient {
    async fn fetch_issues(&self, _owner: &str, _repo: &str) -> Result<Vec<GhIssue>> {
        Ok(self.issues.clone())
    }

    async fn fetch_issue_comments(
        &self,
        _owner: &str,
        _repo: &str,
        number: u64,
    ) -> Result<Vec<GhIssueComment>> {
        Ok(self
            .comments_by_issue
            .get(&number)
            .cloned()
            .unwrap_or_default())
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
        Ok(1)
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

fn gh_comment(id: u64, body: &str) -> GhIssueComment {
    GhIssueComment {
        id,
        body: Some(body.to_string()),
        user: GhUser {
            login: "commenter".into(),
        },
        created_at: "2025-01-02T00:00:00Z".into(),
    }
}

fn gh_comment_empty_body(id: u64) -> GhIssueComment {
    GhIssueComment {
        id,
        body: None,
        user: GhUser {
            login: "commenter".into(),
        },
        created_at: "2025-01-02T00:00:00Z".into(),
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[tokio::test]
async fn import_issue_comments_adds_chain() {
    let (_dir, repo) = test_repo();
    let cfg = test_config();
    let mut comments_map = HashMap::new();
    comments_map.insert(
        1u64,
        vec![
            gh_comment(10, "first comment"),
            gh_comment(11, "second comment"),
        ],
    );
    let client = CommentMockClient::new(vec![gh_issue(1, "Bug")], comments_map);

    let report = import_issues(&repo, &cfg, &client).await.unwrap();
    assert_eq!(report.imported, 1 + 2); // 1 issue + 2 comments

    let store = Store::new(&repo);
    let issue = store.get_issue("GH#1").unwrap();
    let ref_name = issue_comment_ref(&issue.oid);
    let comments = list_comments(&repo, &ref_name).unwrap();
    assert_eq!(comments.len(), 2);
}

#[tokio::test]
async fn import_comment_skips_already_imported() {
    let (_dir, repo) = test_repo();
    let cfg = test_config();
    let mut comments_map = HashMap::new();
    comments_map.insert(1u64, vec![gh_comment(10, "a comment")]);
    let client = CommentMockClient::new(vec![gh_issue(1, "Bug")], comments_map);

    let r1 = import_issues(&repo, &cfg, &client).await.unwrap();
    assert_eq!(r1.imported, 2); // 1 issue + 1 comment

    let r2 = import_issues(&repo, &cfg, &client).await.unwrap();
    // Issue skipped, comment skipped.
    assert_eq!(r2.skipped, 1 + 1);
    assert_eq!(r2.imported, 0);
}

#[tokio::test]
async fn export_issue_comment_creates_github_comment() {
    let (_dir, repo) = test_repo();
    let cfg = test_config();
    let client = CommentMockClient::new(Vec::new(), HashMap::new());

    // Create and export an issue.
    let store = Store::new(&repo);
    let issue = store.create_issue("Local issue", "body", &[], &[]).unwrap();
    export_issues(&repo, &cfg, &client).await.unwrap();

    // Add a local comment.
    let ref_name = issue_comment_ref(&issue.oid);
    let comment = add_comment(&repo, &ref_name, "my comment", None).unwrap();

    // Re-export: should pick up the new comment.
    let report = export_issues(&repo, &cfg, &client).await.unwrap();
    assert_eq!(report.exported, 1); // 1 comment exported

    // Sync state should record comments/100 → chain_oid.
    let state = forge_github::state::load_sync_state(&repo, &cfg.owner, &cfg.repo).unwrap();
    assert_eq!(
        state.get("comments/100").map(String::as_str),
        Some(comment.oid.as_str())
    );

    // The mock should have received the comment body.
    let created = client.created_comments.borrow();
    assert_eq!(created.len(), 1);
    assert_eq!(created[0].1, "my comment");
}

#[tokio::test]
async fn export_comment_skips_already_exported() {
    let (_dir, repo) = test_repo();
    let cfg = test_config();
    let client = CommentMockClient::new(Vec::new(), HashMap::new());

    let store = Store::new(&repo);
    let issue = store.create_issue("Local issue", "body", &[], &[]).unwrap();
    export_issues(&repo, &cfg, &client).await.unwrap();

    let ref_name = issue_comment_ref(&issue.oid);
    add_comment(&repo, &ref_name, "my comment", None).unwrap();

    let r1 = export_issues(&repo, &cfg, &client).await.unwrap();
    assert_eq!(r1.exported, 1);

    let r2 = export_issues(&repo, &cfg, &client).await.unwrap();
    assert_eq!(r2.exported, 0);
    assert_eq!(r2.skipped, 1 + 1); // issue + comment both skipped
}

#[tokio::test]
async fn roundtrip_comments_no_duplicates() {
    let (_dir, repo) = test_repo();
    let cfg = test_config();

    // First: export a local issue and comment.
    let export_client = CommentMockClient::new(Vec::new(), HashMap::new());
    let store = Store::new(&repo);
    let issue = store.create_issue("Bug", "body", &[], &[]).unwrap();
    export_issues(&repo, &cfg, &export_client).await.unwrap();

    let ref_name = issue_comment_ref(&issue.oid);
    add_comment(&repo, &ref_name, "a comment", None).unwrap();
    export_issues(&repo, &cfg, &export_client).await.unwrap();

    // Now import: the comment from GitHub would have the same content.
    // The dedup is by chain OID in the export state, not by content.
    // Re-import with a mock that returns the exported issue + a GitHub comment.
    let mut comments_map = HashMap::new();
    comments_map.insert(1u64, vec![gh_comment(999, "unrelated github comment")]);
    let import_client = CommentMockClient::new(vec![gh_issue(1, "Bug")], comments_map);
    let report = import_issues(&repo, &cfg, &import_client).await.unwrap();
    // Issue already in state → skipped. GitHub comment is new → imported.
    assert_eq!(report.skipped, 1);
    assert_eq!(report.imported, 1); // 1 new github comment

    // Verify only one extra chain entry (the imported github comment).
    let comments = list_comments(&repo, &ref_name).unwrap();
    assert_eq!(comments.len(), 2); // our local + the imported one
}

#[tokio::test]
async fn import_comments_for_new_issue_in_same_pass() {
    let (_dir, repo) = test_repo();
    let cfg = test_config();
    let mut comments_map = HashMap::new();
    comments_map.insert(5u64, vec![gh_comment(50, "comment on new issue")]);
    let client = CommentMockClient::new(vec![gh_issue(5, "Fresh issue")], comments_map);

    // Single import pass should create both the issue and its comments.
    let report = import_issues(&repo, &cfg, &client).await.unwrap();
    assert_eq!(report.imported, 2); // 1 issue + 1 comment

    let store = Store::new(&repo);
    let issue = store.get_issue("GH#5").unwrap();
    let ref_name = issue_comment_ref(&issue.oid);
    let comments = list_comments(&repo, &ref_name).unwrap();
    assert_eq!(comments.len(), 1);
    assert!(comments[0].body.contains("comment on new issue"));
}

#[tokio::test]
async fn import_comment_with_empty_body() {
    let (_dir, repo) = test_repo();
    let cfg = test_config();
    let mut comments_map = HashMap::new();
    comments_map.insert(1u64, vec![gh_comment_empty_body(20)]);
    let client = CommentMockClient::new(vec![gh_issue(1, "Bug")], comments_map);

    let report = import_issues(&repo, &cfg, &client).await.unwrap();
    assert_eq!(report.imported, 2); // 1 issue + 1 comment

    let store = Store::new(&repo);
    let issue = store.get_issue("GH#1").unwrap();
    let ref_name = issue_comment_ref(&issue.oid);
    let comments = list_comments(&repo, &ref_name).unwrap();
    assert_eq!(comments.len(), 1);
    // Empty body should only produce the Github-Id trailer, which is parsed out.
    assert!(comments[0].body.is_empty());
}

#[tokio::test]
async fn standalone_import_issue_comments() {
    let (_dir, repo) = test_repo();
    let cfg = test_config();
    // First import the issue so state has it.
    let client = CommentMockClient::new(vec![gh_issue(1, "Bug")], HashMap::new());
    import_issues(&repo, &cfg, &client).await.unwrap();

    // Now add comments via the standalone function.
    let mut comments_map = HashMap::new();
    comments_map.insert(1u64, vec![gh_comment(30, "standalone import")]);
    let client2 = CommentMockClient::new(Vec::new(), comments_map);
    let report = import_issue_comments(&repo, &cfg, &client2, 1)
        .await
        .unwrap();
    assert_eq!(report.imported, 1);

    let store = Store::new(&repo);
    let issue = store.get_issue("GH#1").unwrap();
    let ref_name = issue_comment_ref(&issue.oid);
    let comments = list_comments(&repo, &ref_name).unwrap();
    assert_eq!(comments.len(), 1);
    assert!(comments[0].body.contains("standalone import"));
}

#[tokio::test]
async fn standalone_import_comments_missing_issue_is_noop() {
    let (_dir, repo) = test_repo();
    let cfg = test_config();
    let client = CommentMockClient::new(Vec::new(), HashMap::new());

    // Issue 99 was never imported, so this should be a no-op.
    let report = import_issue_comments(&repo, &cfg, &client, 99)
        .await
        .unwrap();
    assert_eq!(report.imported, 0);
    assert_eq!(report.skipped, 0);
    assert_eq!(report.failed, 0);
}

#[tokio::test]
async fn standalone_export_issue_comments() {
    let (_dir, repo) = test_repo();
    let cfg = test_config();
    let client = CommentMockClient::new(Vec::new(), HashMap::new());

    // Create and export an issue first.
    let store = Store::new(&repo);
    let issue = store.create_issue("Local", "body", &[], &[]).unwrap();
    export_issues(&repo, &cfg, &client).await.unwrap();

    // Add a comment and export via standalone function.
    let ref_name = issue_comment_ref(&issue.oid);
    add_comment(&repo, &ref_name, "standalone export", None).unwrap();

    let report = export_issue_comments(&repo, &cfg, &client, &issue.oid)
        .await
        .unwrap();
    assert_eq!(report.exported, 1);

    let created = client.created_comments.borrow();
    assert_eq!(created.len(), 1);
    assert_eq!(created[0].1, "standalone export");
}

#[tokio::test]
async fn imported_comment_body_strips_github_id_trailer() {
    let (_dir, repo) = test_repo();
    let cfg = test_config();
    let mut comments_map = HashMap::new();
    comments_map.insert(1u64, vec![gh_comment(10, "hello from github")]);
    let client = CommentMockClient::new(vec![gh_issue(1, "Bug")], comments_map);

    import_issues(&repo, &cfg, &client).await.unwrap();

    let store = Store::new(&repo);
    let issue = store.get_issue("GH#1").unwrap();
    let ref_name = issue_comment_ref(&issue.oid);
    let comments = list_comments(&repo, &ref_name).unwrap();
    assert_eq!(comments.len(), 1);
    // The stored message includes "Github-Id: 10" as a trailer, but
    // list_comments must strip it so consumers see only the original body.
    assert_eq!(comments[0].body, "hello from github");
}

#[tokio::test]
async fn standalone_export_comments_missing_issue_is_noop() {
    let (_dir, repo) = test_repo();
    let cfg = test_config();
    let client = CommentMockClient::new(Vec::new(), HashMap::new());

    let report = export_issue_comments(&repo, &cfg, &client, "nonexistent_oid")
        .await
        .unwrap();
    assert_eq!(report.exported, 0);
    assert_eq!(report.skipped, 0);
    assert_eq!(report.failed, 0);
}
