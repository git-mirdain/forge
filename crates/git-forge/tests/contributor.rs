//! Integration tests for the contributor model.
#![allow(clippy::must_use_candidate, clippy::missing_panics_doc, missing_docs)]

use git_forge::Store;
use git_forge::contributor::{ContributorId, Handle};
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

// ---------------------------------------------------------------------------
// Handle validation
// ---------------------------------------------------------------------------

#[test]
fn handle_rejects_empty() {
    assert!(Handle::new("").is_err());
}

#[test]
fn handle_rejects_slash() {
    assert!(Handle::new("foo/bar").is_err());
}

#[test]
fn handle_rejects_whitespace() {
    assert!(Handle::new("foo bar").is_err());
}

#[test]
fn handle_accepts_valid() {
    assert!(Handle::new("alice").is_ok());
    assert!(Handle::new("alice-smith").is_ok());
    assert!(Handle::new("alice_smith").is_ok());
}

// ---------------------------------------------------------------------------
// ContributorId
// ---------------------------------------------------------------------------

#[test]
fn contributor_id_new_is_valid_uuid() {
    let id = ContributorId::new();
    assert!(!id.as_str().is_empty());
    assert!(ContributorId::parse(id.as_str()).is_ok());
}

#[test]
fn contributor_id_parse_rejects_garbage() {
    assert!(ContributorId::parse("not-a-uuid").is_err());
}

// ---------------------------------------------------------------------------
// create / list / get / bootstrap round-trips
// ---------------------------------------------------------------------------

#[test]
fn create_and_list_contributors() {
    let (dir, repo) = test_repo();
    let store = Store::new(&repo);
    let _ = dir;

    store.create_contributor("alice", &[], &[], &[]).unwrap();
    store
        .create_contributor("bob", &[], &[], &["maintainer"])
        .unwrap();

    let contributors = store.list_contributors().unwrap();
    assert_eq!(contributors.len(), 2);

    let handles: Vec<&str> = contributors.iter().map(|c| c.handle.as_str()).collect();
    assert!(handles.contains(&"alice"));
    assert!(handles.contains(&"bob"));
}

#[test]
fn list_contributors_empty_initially() {
    let (dir, repo) = test_repo();
    let store = Store::new(&repo);
    let _ = dir;
    assert!(store.list_contributors().unwrap().is_empty());
}

#[test]
fn get_contributor_by_id() {
    let (dir, repo) = test_repo();
    let store = Store::new(&repo);
    let _ = dir;

    let created = store
        .create_contributor("alice", &[], &[], &["admin"])
        .unwrap();
    let fetched = store.get_contributor(created.id.as_str()).unwrap();
    assert_eq!(fetched.handle.as_str(), "alice");
    assert!(fetched.roles.contains(&"admin".to_string()));
}

#[test]
fn get_contributor_not_found_errors() {
    let (dir, repo) = test_repo();
    let store = Store::new(&repo);
    let _ = dir;

    let id = ContributorId::new();
    assert!(store.get_contributor(id.as_str()).is_err());
}

#[test]
fn duplicate_handle_errors() {
    let (dir, repo) = test_repo();
    let store = Store::new(&repo);
    let _ = dir;

    store.create_contributor("alice", &[], &[], &[]).unwrap();
    assert!(store.create_contributor("alice", &[], &[], &[]).is_err());
}

// ---------------------------------------------------------------------------
// handle / fingerprint resolution
// ---------------------------------------------------------------------------

#[test]
fn resolve_handle_to_uuid() {
    let (dir, repo) = test_repo();
    let store = Store::new(&repo);
    let _ = dir;

    let created = store.create_contributor("alice", &[], &[], &[]).unwrap();
    let resolved = store.resolve_handle("alice").unwrap();
    assert_eq!(resolved, created.id);
}

#[test]
fn resolve_handle_not_found_errors() {
    let (dir, repo) = test_repo();
    let store = Store::new(&repo);
    let _ = dir;

    assert!(store.resolve_handle("ghost").is_err());
}

// ---------------------------------------------------------------------------
// rename
// ---------------------------------------------------------------------------

#[test]
fn rename_contributor() {
    let (dir, repo) = test_repo();
    let store = Store::new(&repo);
    let _ = dir;

    let created = store.create_contributor("alice", &[], &[], &[]).unwrap();
    let renamed = store.rename_contributor("alice", "alicia").unwrap();

    assert_eq!(renamed.id, created.id);
    assert_eq!(renamed.handle.as_str(), "alicia");

    // Old handle is gone.
    assert!(store.resolve_handle("alice").is_err());
    // New handle resolves.
    let resolved = store.resolve_handle("alicia").unwrap();
    assert_eq!(resolved, created.id);
}

#[test]
fn rename_to_taken_handle_errors() {
    let (dir, repo) = test_repo();
    let store = Store::new(&repo);
    let _ = dir;

    store.create_contributor("alice", &[], &[], &[]).unwrap();
    store.create_contributor("bob", &[], &[], &[]).unwrap();

    assert!(store.rename_contributor("alice", "bob").is_err());
}

// ---------------------------------------------------------------------------
// bootstrap
// ---------------------------------------------------------------------------

#[test]
fn bootstrap_creates_admin_contributor() {
    let (dir, repo) = test_repo();
    let store = Store::new(&repo);
    let _ = dir;

    let c = store.bootstrap_contributor().unwrap();
    assert!(c.roles.contains(&"admin".to_string()));
    assert_eq!(store.list_contributors().unwrap().len(), 1);
}

#[test]
fn bootstrap_errors_if_contributors_exist() {
    let (dir, repo) = test_repo();
    let store = Store::new(&repo);
    let _ = dir;

    store.create_contributor("alice", &[], &[], &[]).unwrap();
    assert!(store.bootstrap_contributor().is_err());
}

// ---------------------------------------------------------------------------
// names / emails subtrees
// ---------------------------------------------------------------------------

#[test]
fn create_with_names_and_emails() {
    let (dir, repo) = test_repo();
    let store = Store::new(&repo);
    let _ = dir;

    let c = store
        .create_contributor(
            "alice",
            &["Alice Smith", "A. Smith"],
            &["alice@example.com", "alice@work.com"],
            &[],
        )
        .unwrap();

    assert_eq!(c.names.len(), 2);
    assert!(c.names.contains(&"Alice Smith".to_string()));
    assert!(c.names.contains(&"A. Smith".to_string()));
    assert_eq!(c.emails.len(), 2);
    assert!(c.emails.contains(&"alice@example.com".to_string()));

    let fetched = store.get_contributor(c.id.as_str()).unwrap();
    assert_eq!(fetched.names.len(), 2);
    assert_eq!(fetched.emails.len(), 2);
}

#[test]
fn email_to_contributor_map_uses_emails_subtree() {
    let (dir, repo) = test_repo();
    let store = Store::new(&repo);
    let _ = dir;

    let c = store
        .create_contributor("alice", &[], &["a@x.com", "b@x.com"], &[])
        .unwrap();

    let map = store.email_to_contributor_map().unwrap();
    assert_eq!(map.get("a@x.com"), Some(&c.id));
    assert_eq!(map.get("b@x.com"), Some(&c.id));
    assert_eq!(map.get("c@x.com"), None);
}

#[test]
fn bootstrap_seeds_name_and_email_from_git_config() {
    let (dir, repo) = test_repo();
    let store = Store::new(&repo);
    let _ = dir;

    let c = store.bootstrap_contributor().unwrap();
    assert!(c.names.contains(&"alice".to_string()));
    assert!(c.emails.contains(&"alice@example.com".to_string()));
}

// ---------------------------------------------------------------------------
// slash rejection in subtree entry names
// ---------------------------------------------------------------------------

#[test]
fn add_name_rejects_slash() {
    let (dir, repo) = test_repo();
    let store = Store::new(&repo);
    let _ = dir;

    store.create_contributor("alice", &[], &[], &[]).unwrap();
    assert!(store.add_contributor_name("alice", "foo/bar").is_err());
}

#[test]
fn add_email_rejects_slash() {
    let (dir, repo) = test_repo();
    let store = Store::new(&repo);
    let _ = dir;

    store.create_contributor("alice", &[], &[], &[]).unwrap();
    assert!(store.add_contributor_email("alice", "a/b@x.com").is_err());
}

#[test]
fn add_role_rejects_slash() {
    let (dir, repo) = test_repo();
    let store = Store::new(&repo);
    let _ = dir;

    store.create_contributor("alice", &[], &[], &[]).unwrap();
    assert!(store.add_contributor_role("alice", "admin/super").is_err());
}

#[test]
fn add_key_rejects_slash() {
    let (dir, repo) = test_repo();
    let store = Store::new(&repo);
    let _ = dir;

    store.create_contributor("alice", &[], &[], &[]).unwrap();
    assert!(
        store
            .add_contributor_key("alice", "fp/nested", b"key material")
            .is_err()
    );
}

#[test]
fn add_name_rejects_empty() {
    let (dir, repo) = test_repo();
    let store = Store::new(&repo);
    let _ = dir;

    store.create_contributor("alice", &[], &[], &[]).unwrap();
    assert!(store.add_contributor_name("alice", "").is_err());
}
