use git2::Repository;
use tempfile::TempDir;

use crate::issues::{IssueOpts, Issues};

fn repo() -> (TempDir, Repository) {
    let dir = TempDir::new().unwrap();
    let repo = Repository::init(dir.path()).unwrap();
    {
        let sig = repo.signature().unwrap();
        let tree_oid = repo.treebuilder(None).unwrap().write().unwrap();
        let tree = repo.find_tree(tree_oid).unwrap();
        repo.commit(Some("HEAD"), &sig, &sig, "init", &tree, &[])
            .unwrap();
    }
    (dir, repo)
}

#[test]
fn returns_none_for_missing_issue() {
    let (_dir, repo) = repo();
    assert!(repo.find_issue(1, None).unwrap().is_none());
}

#[test]
fn returns_none_for_nonexistent_id() {
    let (_dir, repo) = repo();
    repo.create_issue("Exists", "", &[], &[], None).unwrap();
    assert!(repo.find_issue(99, None).unwrap().is_none());
}

#[test]
fn finds_existing_issue_by_id() {
    let (_dir, repo) = repo();
    let id = repo
        .create_issue("Find me", "some body", &[], &[], None)
        .unwrap();
    let issue = repo.find_issue(id, None).unwrap().unwrap();
    assert_eq!(issue.id, id);
    assert_eq!(issue.meta.title, "Find me");
    assert_eq!(issue.body, "some body");
}

#[test]
fn finds_correct_issue_among_many() {
    let (_dir, repo) = repo();
    repo.create_issue("First", "", &[], &[], None).unwrap();
    repo.create_issue("Second", "second body", &[], &[], None)
        .unwrap();
    repo.create_issue("Third", "", &[], &[], None).unwrap();
    let issue = repo.find_issue(2, None).unwrap().unwrap();
    assert_eq!(issue.id, 2);
    assert_eq!(issue.meta.title, "Second");
    assert_eq!(issue.body, "second body");
}

#[test]
fn respects_custom_ref_prefix() {
    let (_dir, repo) = repo();
    let opts = IssueOpts {
        ref_prefix: "refs/alt-issues/".to_string(),
    };
    let id = repo.create_issue("Alt", "", &[], &[], Some(&opts)).unwrap();
    // Not visible under default prefix.
    assert!(repo.find_issue(id, None).unwrap().is_none());
    // Visible under custom prefix.
    let issue = repo.find_issue(id, Some(&opts)).unwrap().unwrap();
    assert_eq!(issue.meta.title, "Alt");
}
