use git2::Repository;
use tempfile::TempDir;

use crate::issues::{IssueOpts, IssueState, Issues};

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
fn assigns_id_one_for_first_issue() {
    let (_dir, repo) = repo();
    let id = repo.create_issue("First", "body", &[], &[], None).unwrap();
    assert_eq!(id, 1);
}

#[test]
fn assigns_sequential_ids() {
    let (_dir, repo) = repo();
    let a = repo.create_issue("A", "", &[], &[], None).unwrap();
    let b = repo.create_issue("B", "", &[], &[], None).unwrap();
    let c = repo.create_issue("C", "", &[], &[], None).unwrap();
    assert_eq!((a, b, c), (1, 2, 3));
}

#[test]
fn stores_title_and_body() {
    let (_dir, repo) = repo();
    repo.create_issue("My title", "My body", &[], &[], None)
        .unwrap();
    let issue = repo.find_issue(1, None).unwrap().unwrap();
    assert_eq!(issue.meta.title, "My title");
    assert_eq!(issue.body, "My body");
}

#[test]
fn new_issue_is_open() {
    let (_dir, repo) = repo();
    repo.create_issue("Open me", "", &[], &[], None).unwrap();
    let issue = repo.find_issue(1, None).unwrap().unwrap();
    assert_eq!(issue.meta.state, IssueState::Open);
}

#[test]
fn stores_labels() {
    let (_dir, repo) = repo();
    let labels = vec!["bug".to_string(), "help wanted".to_string()];
    repo.create_issue("Labeled", "", &labels, &[], None)
        .unwrap();
    let issue = repo.find_issue(1, None).unwrap().unwrap();
    let mut got = issue.meta.labels.clone();
    got.sort();
    let mut expected = labels.clone();
    expected.sort();
    assert_eq!(got, expected);
}

#[test]
fn no_labels_gives_empty_vec() {
    let (_dir, repo) = repo();
    repo.create_issue("No labels", "", &[], &[], None).unwrap();
    let issue = repo.find_issue(1, None).unwrap().unwrap();
    assert!(issue.meta.labels.is_empty());
}

#[test]
fn custom_ref_prefix() {
    let (_dir, repo) = repo();
    let opts = IssueOpts {
        ref_prefix: "refs/test-issues/".to_string(),
    };
    let id = repo
        .create_issue("Prefixed", "", &[], &[], Some(&opts))
        .unwrap();
    assert_eq!(id, 1);
    // Should not appear under the default prefix.
    assert!(repo.find_issue(1, None).unwrap().is_none());
    // Should appear under the custom prefix.
    let issue = repo.find_issue(1, Some(&opts)).unwrap().unwrap();
    assert_eq!(issue.meta.title, "Prefixed");
}
