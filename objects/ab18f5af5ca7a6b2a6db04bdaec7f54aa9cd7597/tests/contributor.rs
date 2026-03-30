//! Integration tests for contributor config and review-approval wiring.
#![allow(clippy::must_use_candidate, clippy::missing_panics_doc, missing_docs)]

use std::path::Path;

use git_forge::Store;
use git_forge::exe::Executor;
use git_forge::review::ReviewTarget;
use git2::Repository;
use tempfile::TempDir;

fn test_repo() -> (TempDir, Repository) {
    let dir = TempDir::new().expect("temp dir");
    let repo = Repository::init(dir.path()).expect("init repo");
    {
        let mut cfg = repo.config().expect("config");
        cfg.set_str("user.name", "alice").expect("user.name");
        cfg.set_str("user.email", "alice@example.com")
            .expect("user.email");
    }
    {
        let sig = git2::Signature::now("alice", "alice@example.com").expect("sig");
        let mut index = repo.index().expect("index");
        let tree_oid = index.write_tree().expect("write tree");
        let tree = repo.find_tree(tree_oid).expect("find tree");
        repo.commit(Some("HEAD"), &sig, &sig, "initial", &tree, &[])
            .expect("initial commit");
    }
    (dir, repo)
}

fn executor(path: &Path) -> Executor {
    Executor::from_path(path).expect("executor")
}

fn head_oid(repo: &Repository) -> String {
    repo.head()
        .unwrap()
        .peel_to_commit()
        .unwrap()
        .id()
        .to_string()
}

// ---------------------------------------------------------------------------
// contributor_add / contributor_list round-trip
// ---------------------------------------------------------------------------

#[test]
fn add_and_list_contributor() {
    let (dir, _repo) = test_repo();
    let exe = executor(dir.path());

    exe.contributor_add("alice", &["alice@example.com"], &["Alice"])
        .unwrap();

    let contributors = exe.contributor_list().unwrap();
    assert_eq!(contributors.len(), 1);
    assert_eq!(contributors[0].id, "alice");
    assert_eq!(contributors[0].emails, vec!["alice@example.com"]);
    assert_eq!(contributors[0].names, vec!["Alice"]);
}

#[test]
fn add_contributor_is_additive() {
    let (dir, _repo) = test_repo();
    let exe = executor(dir.path());

    exe.contributor_add("alice", &["alice@example.com"], &["Alice"])
        .unwrap();
    exe.contributor_add("alice", &["alice@work.com"], &["A. Smith"])
        .unwrap();

    let contributors = exe.contributor_list().unwrap();
    assert_eq!(contributors.len(), 1);
    assert_eq!(contributors[0].emails.len(), 2);
    assert!(
        contributors[0]
            .emails
            .contains(&"alice@example.com".to_string())
    );
    assert!(
        contributors[0]
            .emails
            .contains(&"alice@work.com".to_string())
    );
    assert_eq!(contributors[0].names.len(), 2);
}

#[test]
fn list_contributors_empty_without_config() {
    let (dir, _repo) = test_repo();
    let exe = executor(dir.path());

    let contributors = exe.contributor_list().unwrap();
    assert!(contributors.is_empty());
}

#[test]
fn multiple_contributors() {
    let (dir, _repo) = test_repo();
    let exe = executor(dir.path());

    exe.contributor_add("alice", &["alice@example.com"], &["Alice"])
        .unwrap();
    exe.contributor_add("bob", &["bob@example.com"], &["Bob"])
        .unwrap();

    let contributors = exe.contributor_list().unwrap();
    assert_eq!(contributors.len(), 2);
    let ids: Vec<&str> = contributors.iter().map(|c| c.id.as_str()).collect();
    assert!(ids.contains(&"alice"));
    assert!(ids.contains(&"bob"));
}

// ---------------------------------------------------------------------------
// contributor_remove
// ---------------------------------------------------------------------------

#[test]
fn remove_contributor() {
    let (dir, _repo) = test_repo();
    let exe = executor(dir.path());

    exe.contributor_add("alice", &["alice@example.com"], &["Alice"])
        .unwrap();
    exe.contributor_add("bob", &["bob@example.com"], &["Bob"])
        .unwrap();

    exe.contributor_remove("alice").unwrap();

    let contributors = exe.contributor_list().unwrap();
    assert_eq!(contributors.len(), 1);
    assert_eq!(contributors[0].id, "bob");
}

#[test]
fn remove_nonexistent_contributor_errors() {
    let (dir, _repo) = test_repo();
    let exe = executor(dir.path());

    let result = exe.contributor_remove("ghost");
    assert!(result.is_err());
}

// ---------------------------------------------------------------------------
// review approval auto-registers contributor
// ---------------------------------------------------------------------------

#[test]
fn approve_review_registers_contributor() {
    let (dir, repo) = test_repo();
    let exe = executor(dir.path());
    let store = Store::new(&repo);

    let target = ReviewTarget {
        head: head_oid(&repo),
        base: None,
    };
    let review = store.create_review("Review me", "", &target, None).unwrap();

    // No contributors yet.
    assert!(exe.contributor_list().unwrap().is_empty());

    // Approving auto-registers the current user.
    exe.approve_review(&review.oid, Some("lgtm")).unwrap();

    let contributors = exe.contributor_list().unwrap();
    assert_eq!(contributors.len(), 1);
    assert_eq!(contributors[0].id, "alice");
    assert!(
        contributors[0]
            .emails
            .contains(&"alice@example.com".to_string())
    );
}

#[test]
fn approve_review_skips_if_already_registered() {
    let (dir, repo) = test_repo();
    let exe = executor(dir.path());
    let store = Store::new(&repo);

    // Pre-register the contributor.
    exe.contributor_add("alice", &["alice@example.com"], &["Alice"])
        .unwrap();

    let target = ReviewTarget {
        head: head_oid(&repo),
        base: None,
    };
    let review = store.create_review("Review me", "", &target, None).unwrap();

    // Approve — should not duplicate the contributor.
    exe.approve_review(&review.oid, Some("lgtm")).unwrap();

    let contributors = exe.contributor_list().unwrap();
    assert_eq!(contributors.len(), 1);
    assert_eq!(contributors[0].emails.len(), 1);
}
