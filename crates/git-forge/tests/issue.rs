//! Integration tests for `Store` issue CRUD.
#![allow(clippy::must_use_candidate, clippy::missing_panics_doc, missing_docs)]

use git_forge::issue::IssueState;
use git_forge::refs::ISSUE_INDEX;
use git_forge::{Error, Store};
use git2::Repository;
use tempfile::TempDir;

fn test_repo() -> (TempDir, Repository) {
    let dir = TempDir::new().expect("temp dir");
    let repo = Repository::init(dir.path()).expect("init repo");
    // Set identity so commits succeed regardless of global git config.
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

// ---------------------------------------------------------------------------
// create_issue
// ---------------------------------------------------------------------------

#[test]
fn create_issue_basic_fields() {
    let (_dir, repo) = test_repo();
    let store = Store::new(&repo);
    let issue = store
        .create_issue("Fix the bug", "some details", &[], &[])
        .unwrap();

    assert_eq!(issue.title, "Fix the bug");
    assert_eq!(issue.body, "some details");
    assert_eq!(issue.state, IssueState::Open);
    assert!(issue.labels.is_empty());
    assert!(issue.assignees.is_empty());
    assert_eq!(issue.display_id, None); // not set until synced
    assert_eq!(issue.oid.len(), 40);
    assert!(issue.oid.chars().all(|c| c.is_ascii_hexdigit()));

    // Read back from git to verify persistence, not just the returned struct.
    let fetched = store.get_issue(&issue.oid).unwrap();
    assert_eq!(fetched.title, "Fix the bug");
    assert_eq!(fetched.body, "some details");
    assert_eq!(fetched.state, IssueState::Open);
}

#[test]
fn create_issue_with_labels_and_assignees() {
    let (_dir, repo) = test_repo();
    let store = Store::new(&repo);
    let issue = store
        .create_issue("Crash", "oops", &["bug", "critical"], &["alice", "bob"])
        .unwrap();

    let mut labels = issue.labels.clone();
    labels.sort();
    assert_eq!(labels, vec!["bug", "critical"]);

    let mut assignees = issue.assignees.clone();
    assignees.sort();
    assert_eq!(assignees, vec!["alice", "bob"]);
}

#[test]
fn create_issue_ref_exists_in_repo() {
    let (_dir, repo) = test_repo();
    let store = Store::new(&repo);
    let issue = store.create_issue("Ref test", "", &[], &[]).unwrap();

    let ref_name = format!("refs/forge/issue/{}", issue.oid);
    assert!(repo.find_reference(&ref_name).is_ok());
}

#[test]
fn create_two_issues_have_distinct_oids() {
    let (_dir, repo) = test_repo();
    let store = Store::new(&repo);
    let a = store.create_issue("First", "body", &[], &[]).unwrap();
    let b = store.create_issue("Second", "body", &[], &[]).unwrap();
    assert_ne!(a.oid, b.oid);
}

#[test]
fn create_issue_unicode_fields() {
    let (_dir, repo) = test_repo();
    let store = Store::new(&repo);
    let issue = store
        .create_issue("日本語タイトル", "内容：テスト", &["バグ"], &["ユーザー"])
        .unwrap();

    let fetched = store.get_issue(&issue.oid).unwrap();
    assert_eq!(fetched.title, "日本語タイトル");
    assert_eq!(fetched.body, "内容：テスト");
    assert_eq!(fetched.labels, vec!["バグ"]);
    assert_eq!(fetched.assignees, vec!["ユーザー"]);
}

// ---------------------------------------------------------------------------
// get_issue
// ---------------------------------------------------------------------------

#[test]
fn get_issue_by_full_oid() {
    let (_dir, repo) = test_repo();
    let store = Store::new(&repo);
    let created = store.create_issue("Title", "body", &[], &[]).unwrap();

    let fetched = store.get_issue(&created.oid).unwrap();
    assert_eq!(fetched.oid, created.oid);
    assert_eq!(fetched.title, "Title");
    assert_eq!(fetched.body, "body");
}

#[test]
fn get_issue_by_oid_prefix() {
    let (_dir, repo) = test_repo();
    let store = Store::new(&repo);
    let created = store.create_issue("Title", "body", &[], &[]).unwrap();

    let prefix = &created.oid[..8];
    let fetched = store.get_issue(prefix).unwrap();
    assert_eq!(fetched.oid, created.oid);
}

#[test]
fn get_issue_by_display_id() {
    let (_dir, repo) = test_repo();
    let store = Store::new(&repo);
    let created = store.create_issue("Title", "body", &[], &[]).unwrap();
    store
        .write_display_id(ISSUE_INDEX, "MY#1", &created.oid)
        .unwrap();

    let fetched = store.get_issue("MY#1").unwrap();
    assert_eq!(fetched.oid, created.oid);
    assert_eq!(fetched.display_id, Some("MY#1".to_string()));
}

#[test]
fn get_issue_not_found_non_hex() {
    let (_dir, repo) = test_repo();
    let store = Store::new(&repo);

    let err = store.get_issue("nonexistent").unwrap_err();
    assert!(matches!(err, Error::NotFound(_)));
}

#[test]
fn get_issue_not_found_hex_prefix_no_match() {
    let (_dir, repo) = test_repo();
    let store = Store::new(&repo);

    // A hex string that won't match any entity OID.
    let err = store.get_issue("0000000000000000").unwrap_err();
    assert!(matches!(err, Error::NotFound(_)));
}

#[test]
fn get_issue_returns_correct_display_id_from_index() {
    let (_dir, repo) = test_repo();
    let store = Store::new(&repo);
    let author = git2::Signature::now("gh", "gh@example.com").unwrap();
    let created = store
        .create_issue_imported(
            "Imported",
            "body",
            &[],
            &[],
            "GH#42",
            &author,
            "https://example.com",
        )
        .unwrap();

    // get_issue by OID should still surface the display ID.
    let fetched = store.get_issue(&created.oid).unwrap();
    assert_eq!(fetched.display_id, Some("GH#42".to_string()));
}

// ---------------------------------------------------------------------------
// list_issues
// ---------------------------------------------------------------------------

#[test]
fn list_issues_empty_repo() {
    let (_dir, repo) = test_repo();
    let store = Store::new(&repo);
    let issues = store.list_issues().unwrap();
    assert!(issues.is_empty());
}

#[test]
fn list_issues_returns_all() {
    let (_dir, repo) = test_repo();
    let store = Store::new(&repo);
    store.create_issue("Alpha", "a", &[], &[]).unwrap();
    store.create_issue("Beta", "b", &[], &[]).unwrap();
    store.create_issue("Gamma", "c", &[], &[]).unwrap();

    let issues = store.list_issues().unwrap();
    assert_eq!(issues.len(), 3);

    let mut titles: Vec<&str> = issues.iter().map(|i| i.title.as_str()).collect();
    titles.sort_unstable();
    assert_eq!(titles, vec!["Alpha", "Beta", "Gamma"]);
}

#[test]
fn list_issues_fields_round_trip() {
    let (_dir, repo) = test_repo();
    let store = Store::new(&repo);
    store
        .create_issue("Title", "body text", &["label-a", "label-b"], &["user1"])
        .unwrap();

    let issues = store.list_issues().unwrap();
    assert_eq!(issues.len(), 1);

    let issue = &issues[0];
    assert_eq!(issue.title, "Title");
    assert_eq!(issue.body, "body text");

    let mut labels = issue.labels.clone();
    labels.sort();
    assert_eq!(labels, vec!["label-a", "label-b"]);
    assert_eq!(issue.assignees, vec!["user1"]);
}

// ---------------------------------------------------------------------------
// list_issues_by_state
// ---------------------------------------------------------------------------

#[test]
fn list_by_state_empty_repo() {
    let (_dir, repo) = test_repo();
    let store = Store::new(&repo);
    assert!(
        store
            .list_issues_by_state(&IssueState::Open)
            .unwrap()
            .is_empty()
    );
    assert!(
        store
            .list_issues_by_state(&IssueState::Closed)
            .unwrap()
            .is_empty()
    );
}

#[test]
fn list_by_state_filters_correctly() {
    let (_dir, repo) = test_repo();
    let store = Store::new(&repo);
    let to_close = store.create_issue("Close me", "", &[], &[]).unwrap();
    store.create_issue("Keep open", "", &[], &[]).unwrap();

    store
        .update_issue(
            &to_close.oid,
            None,
            None,
            Some(&IssueState::Closed),
            &[],
            &[],
            &[],
            &[],
            None,
        )
        .unwrap();

    let open = store.list_issues_by_state(&IssueState::Open).unwrap();
    let closed = store.list_issues_by_state(&IssueState::Closed).unwrap();

    assert_eq!(open.len(), 1);
    assert_eq!(open[0].title, "Keep open");
    assert_eq!(closed.len(), 1);
    assert_eq!(closed[0].title, "Close me");
}

#[test]
fn list_by_state_all_open_by_default() {
    let (_dir, repo) = test_repo();
    let store = Store::new(&repo);
    for i in 0..5 {
        store
            .create_issue(&format!("Issue {i}"), "", &[], &[])
            .unwrap();
    }

    let open = store.list_issues_by_state(&IssueState::Open).unwrap();
    let closed = store.list_issues_by_state(&IssueState::Closed).unwrap();
    assert_eq!(open.len(), 5);
    assert!(closed.is_empty());
}

// ---------------------------------------------------------------------------
// update_issue
// ---------------------------------------------------------------------------

#[test]
fn update_issue_title() {
    let (_dir, repo) = test_repo();
    let store = Store::new(&repo);
    let created = store.create_issue("Old title", "body", &[], &[]).unwrap();

    let updated = store
        .update_issue(
            &created.oid,
            Some("New title"),
            None,
            None,
            &[],
            &[],
            &[],
            &[],
            None,
        )
        .unwrap();

    assert_eq!(updated.title, "New title");
    assert_eq!(updated.body, "body"); // unchanged
}

#[test]
fn update_issue_body() {
    let (_dir, repo) = test_repo();
    let store = Store::new(&repo);
    let created = store.create_issue("Title", "old body", &[], &[]).unwrap();

    let updated = store
        .update_issue(
            &created.oid,
            None,
            Some("new body"),
            None,
            &[],
            &[],
            &[],
            &[],
            None,
        )
        .unwrap();

    assert_eq!(updated.body, "new body");
    assert_eq!(updated.title, "Title"); // unchanged
}

#[test]
fn update_issue_state_open_to_closed() {
    let (_dir, repo) = test_repo();
    let store = Store::new(&repo);
    let created = store.create_issue("Title", "", &[], &[]).unwrap();
    assert_eq!(created.state, IssueState::Open);

    let updated = store
        .update_issue(
            &created.oid,
            None,
            None,
            Some(&IssueState::Closed),
            &[],
            &[],
            &[],
            &[],
            None,
        )
        .unwrap();
    assert_eq!(updated.state, IssueState::Closed);

    // Verify persisted via a fresh read.
    assert_eq!(
        store.get_issue(&created.oid).unwrap().state,
        IssueState::Closed
    );
}

#[test]
fn update_issue_add_label() {
    let (_dir, repo) = test_repo();
    let store = Store::new(&repo);
    let created = store.create_issue("Title", "", &["existing"], &[]).unwrap();

    let updated = store
        .update_issue(
            &created.oid,
            None,
            None,
            None,
            &["new-label"],
            &[],
            &[],
            &[],
            None,
        )
        .unwrap();

    let mut labels = updated.labels.clone();
    labels.sort();
    assert_eq!(labels, vec!["existing", "new-label"]);
}

#[test]
fn update_issue_remove_label() {
    let (_dir, repo) = test_repo();
    let store = Store::new(&repo);
    let created = store
        .create_issue("Title", "", &["keep", "remove-me"], &[])
        .unwrap();

    let updated = store
        .update_issue(
            &created.oid,
            None,
            None,
            None,
            &[],
            &["remove-me"],
            &[],
            &[],
            None,
        )
        .unwrap();

    assert_eq!(updated.labels, vec!["keep"]);
}

#[test]
fn update_issue_remove_nonexistent_label_is_ok() {
    let (_dir, repo) = test_repo();
    let store = Store::new(&repo);
    let created = store.create_issue("Title", "", &["bug"], &[]).unwrap();

    // Removing a label that was never there should not error.
    let updated = store
        .update_issue(
            &created.oid,
            None,
            None,
            None,
            &[],
            &["phantom"],
            &[],
            &[],
            None,
        )
        .unwrap();

    assert_eq!(updated.labels, vec!["bug"]);
}

#[test]
fn update_issue_add_and_remove_assignees() {
    let (_dir, repo) = test_repo();
    let store = Store::new(&repo);
    let created = store
        .create_issue("Title", "", &[], &["alice", "bob"])
        .unwrap();

    let updated = store
        .update_issue(
            &created.oid,
            None,
            None,
            None,
            &[],
            &[],
            &["carol"],
            &["bob"],
            None,
        )
        .unwrap();

    let mut assignees = updated.assignees.clone();
    assignees.sort();
    assert_eq!(assignees, vec!["alice", "carol"]);
}

#[test]
fn update_issue_all_mutations_at_once() {
    let (_dir, repo) = test_repo();
    let store = Store::new(&repo);
    let created = store
        .create_issue("Old", "old body", &["bug"], &["alice"])
        .unwrap();

    let updated = store
        .update_issue(
            &created.oid,
            Some("New"),
            Some("new body"),
            Some(&IssueState::Closed),
            &["feature"],
            &["bug"],
            &["bob"],
            &["alice"],
            None,
        )
        .unwrap();

    assert_eq!(updated.title, "New");
    assert_eq!(updated.body, "new body");
    assert_eq!(updated.state, IssueState::Closed);
    assert_eq!(updated.labels, vec!["feature"]);
    assert_eq!(updated.assignees, vec!["bob"]);
}

#[test]
fn update_issue_not_found() {
    let (_dir, repo) = test_repo();
    let store = Store::new(&repo);

    let err = store
        .update_issue(
            "nonexistent",
            Some("title"),
            None,
            None,
            &[],
            &[],
            &[],
            &[],
            None,
        )
        .unwrap_err();
    assert!(matches!(err, Error::NotFound(_)));
}

#[test]
fn update_issue_no_op_mutation_preserves_fields() {
    let (_dir, repo) = test_repo();
    let store = Store::new(&repo);
    let created = store
        .create_issue("Title", "body", &["bug"], &["alice"])
        .unwrap();

    // All Nones and empty slices — should be a no-op.
    let updated = store
        .update_issue(&created.oid, None, None, None, &[], &[], &[], &[], None)
        .unwrap();

    assert_eq!(updated.title, "Title");
    assert_eq!(updated.body, "body");
    assert_eq!(updated.state, IssueState::Open);
    assert_eq!(updated.labels, vec!["bug"]);
    assert_eq!(updated.assignees, vec!["alice"]);
}

// ---------------------------------------------------------------------------
// create_issue_imported
// ---------------------------------------------------------------------------

#[test]
fn imported_issue_indexed_immediately() {
    let (_dir, repo) = test_repo();
    let store = Store::new(&repo);
    let author = git2::Signature::now("gh-user", "gh-user@users.noreply.github.com").unwrap();

    let created = store
        .create_issue_imported(
            "GH bug",
            "details",
            &["bug"],
            &[],
            "GH#99",
            &author,
            "https://github.com/o/r/issues/99",
        )
        .unwrap();

    assert_eq!(created.display_id, Some("GH#99".to_string()));

    // Resolvable by display ID without a separate write_display_id call.
    let fetched = store.get_issue("GH#99").unwrap();
    assert_eq!(fetched.oid, created.oid);
}

#[test]
fn imported_issue_source_does_not_bleed_into_labels() {
    let (_dir, repo) = test_repo();
    let store = Store::new(&repo);
    let author = git2::Signature::now("u", "u@example.com").unwrap();

    let created = store
        .create_issue_imported(
            "Title",
            "body",
            &[],
            &[],
            "GH#1",
            &author,
            "https://github.com/o/r/issues/1",
        )
        .unwrap();

    assert!(created.labels.is_empty());
    assert!(created.assignees.is_empty());
}

// ---------------------------------------------------------------------------
// write_display_id
// ---------------------------------------------------------------------------

#[test]
fn write_display_id_then_lookup() {
    let (_dir, repo) = test_repo();
    let store = Store::new(&repo);
    let created = store.create_issue("Title", "body", &[], &[]).unwrap();

    store
        .write_display_id(ISSUE_INDEX, "ACME#7", &created.oid)
        .unwrap();

    let fetched = store.get_issue("ACME#7").unwrap();
    assert_eq!(fetched.oid, created.oid);
    assert_eq!(fetched.display_id, Some("ACME#7".to_string()));
}

#[test]
fn write_display_id_overwrites_same_key() {
    let (_dir, repo) = test_repo();
    let store = Store::new(&repo);
    let a = store.create_issue("A", "", &[], &[]).unwrap();
    let b = store.create_issue("B", "", &[], &[]).unwrap();

    // Point X#1 at a, then remap to b.
    store.write_display_id(ISSUE_INDEX, "X#1", &a.oid).unwrap();
    store.write_display_id(ISSUE_INDEX, "X#1", &b.oid).unwrap();

    let fetched = store.get_issue("X#1").unwrap();
    assert_eq!(fetched.oid, b.oid);
}

#[test]
fn write_display_id_multiple_ids_coexist() {
    let (_dir, repo) = test_repo();
    let store = Store::new(&repo);
    let created = store.create_issue("Title", "", &[], &[]).unwrap();

    store
        .write_display_id(ISSUE_INDEX, "GH#1", &created.oid)
        .unwrap();
    store
        .write_display_id(ISSUE_INDEX, "GL#1", &created.oid)
        .unwrap();

    assert_eq!(store.get_issue("GH#1").unwrap().oid, created.oid);
    assert_eq!(store.get_issue("GL#1").unwrap().oid, created.oid);
}

#[test]
fn zero_padded_display_id_resolves() {
    let (_dir, repo) = test_repo();
    let store = Store::new(&repo);
    let created = store.create_issue("Title", "", &[], &[]).unwrap();

    store
        .write_display_id(ISSUE_INDEX, "GH#4", &created.oid)
        .unwrap();

    let fetched = store.get_issue("GH#04").unwrap();
    assert_eq!(fetched.oid, created.oid);
}
