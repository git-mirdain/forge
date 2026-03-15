use git2::Repository;
use tempfile::TempDir;

use crate::issue::{IssueOpts, IssueState, Issues};

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
fn empty_repo_returns_no_issues() {
    let (_dir, repo) = repo();
    let issues = repo.list_issues(None).unwrap();
    assert!(issues.is_empty());
}

#[test]
fn lists_all_issues_ordered_by_id() {
    let (_dir, repo) = repo();
    repo.create_issue("C", "", &[], &[], None).unwrap();
    repo.create_issue("A", "", &[], &[], None).unwrap();
    repo.create_issue("B", "", &[], &[], None).unwrap();
    let issues = repo.list_issues(None).unwrap();
    assert_eq!(issues.len(), 3);
    assert_eq!(issues[0].id, 1);
    assert_eq!(issues[1].id, 2);
    assert_eq!(issues[2].id, 3);
}

#[test]
fn list_includes_correct_titles() {
    let (_dir, repo) = repo();
    repo.create_issue("First", "", &[], &[], None).unwrap();
    repo.create_issue("Second", "", &[], &[], None).unwrap();
    let issues = repo.list_issues(None).unwrap();
    assert_eq!(issues[0].meta.title, "First");
    assert_eq!(issues[1].meta.title, "Second");
}

#[test]
fn list_by_state_open_returns_only_open() {
    let (_dir, repo) = repo();
    repo.create_issue("Open one", "", &[], &[], None).unwrap();
    repo.create_issue("Open two", "", &[], &[], None).unwrap();
    let open = repo.list_issues_by_state(IssueState::Open, None).unwrap();
    assert_eq!(open.len(), 2);
    assert!(open.iter().all(|i| i.meta.state == IssueState::Open));
}

#[test]
fn list_by_state_closed_returns_empty_when_none_closed() {
    let (_dir, repo) = repo();
    repo.create_issue("Open", "", &[], &[], None).unwrap();
    let closed = repo.list_issues_by_state(IssueState::Closed, None).unwrap();
    assert!(closed.is_empty());
}

#[test]
fn list_by_state_results_ordered_by_id() {
    let (_dir, repo) = repo();
    repo.create_issue("One", "", &[], &[], None).unwrap();
    repo.create_issue("Two", "", &[], &[], None).unwrap();
    repo.create_issue("Three", "", &[], &[], None).unwrap();
    let open = repo.list_issues_by_state(IssueState::Open, None).unwrap();
    let ids: Vec<u64> = open.iter().map(|i| i.id).collect();
    assert_eq!(ids, vec![1, 2, 3]);
}

#[test]
fn list_respects_custom_ref_prefix() {
    let (_dir, repo) = repo();
    let opts = IssueOpts {
        ref_prefix: "refs/scoped-issues/".to_string(),
    };
    repo.create_issue("Scoped", "", &[], &[], Some(&opts))
        .unwrap();
    // Default prefix sees nothing.
    assert!(repo.list_issues(None).unwrap().is_empty());
    // Custom prefix sees the issue.
    let issues = repo.list_issues(Some(&opts)).unwrap();
    assert_eq!(issues.len(), 1);
    assert_eq!(issues[0].meta.title, "Scoped");
}

#[test]
fn list_by_state_respects_custom_ref_prefix() {
    let (_dir, repo) = repo();
    let opts = IssueOpts {
        ref_prefix: "refs/scoped-issues/".to_string(),
    };
    repo.create_issue("Scoped open", "", &[], &[], Some(&opts))
        .unwrap();
    let open = repo
        .list_issues_by_state(IssueState::Open, Some(&opts))
        .unwrap();
    assert_eq!(open.len(), 1);
    assert_eq!(open[0].meta.title, "Scoped open");
}
