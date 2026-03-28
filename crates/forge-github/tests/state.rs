//! Integration tests for `forge_github::state` sync-state persistence.
#![allow(clippy::must_use_candidate, clippy::missing_panics_doc, missing_docs)]

use std::collections::HashMap;

use forge_github::state::{
    load_sync_state, lookup_by_forge_oid, lookup_by_github_id, save_sync_state, sync_ref_name,
};
use git2::Repository;
use tempfile::TempDir;

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

// ---------------------------------------------------------------------------
// sync_ref_name
// ---------------------------------------------------------------------------

#[test]
fn sync_ref_name_format() {
    assert_eq!(
        sync_ref_name("my-org", "my-repo"),
        "refs/forge/sync/github/my-org/my-repo"
    );
}

// ---------------------------------------------------------------------------
// load_sync_state — empty / nonexistent
// ---------------------------------------------------------------------------

#[test]
fn load_sync_state_nonexistent_ref_returns_empty() {
    let (_dir, repo) = test_repo();
    let state = load_sync_state(&repo, "org", "repo").unwrap();
    assert!(state.is_empty());
}

// ---------------------------------------------------------------------------
// save / load round-trip
// ---------------------------------------------------------------------------

#[test]
fn save_and_load_roundtrip() {
    let (_dir, repo) = test_repo();

    let mut state = HashMap::new();
    state.insert("issues/1".to_string(), "a".repeat(40));
    state.insert("issues/2".to_string(), "b".repeat(40));

    save_sync_state(&repo, "org", "repo", &state).unwrap();

    let loaded = load_sync_state(&repo, "org", "repo").unwrap();
    assert_eq!(loaded.get("issues/1"), state.get("issues/1"));
    assert_eq!(loaded.get("issues/2"), state.get("issues/2"));
    assert_eq!(loaded.len(), 2);
}

#[test]
fn save_multiple_times_accumulates_state() {
    let (_dir, repo) = test_repo();

    let mut s1 = HashMap::new();
    s1.insert("issues/1".to_string(), "a".repeat(40));
    save_sync_state(&repo, "org", "repo", &s1).unwrap();

    let mut s2 = load_sync_state(&repo, "org", "repo").unwrap();
    s2.insert("issues/2".to_string(), "b".repeat(40));
    save_sync_state(&repo, "org", "repo", &s2).unwrap();

    let loaded = load_sync_state(&repo, "org", "repo").unwrap();
    assert_eq!(loaded.len(), 2);
    assert!(loaded.contains_key("issues/1"));
    assert!(loaded.contains_key("issues/2"));
}

#[test]
fn save_omitting_entry_removes_it_from_subtree() {
    // If the "issues" subtree IS mentioned in the new state, any issue not
    // present in the new state is dropped from that subtree.
    let (_dir, repo) = test_repo();

    let mut state = HashMap::new();
    state.insert("issues/1".to_string(), "a".repeat(40));
    state.insert("issues/2".to_string(), "b".repeat(40));
    save_sync_state(&repo, "org", "repo", &state).unwrap();

    // Re-save with only issues/1 — issues/2 should be gone.
    let mut trimmed = HashMap::new();
    trimmed.insert("issues/1".to_string(), "a".repeat(40));
    save_sync_state(&repo, "org", "repo", &trimmed).unwrap();

    let loaded = load_sync_state(&repo, "org", "repo").unwrap();
    assert_eq!(loaded.len(), 1);
    assert!(loaded.contains_key("issues/1"));
    assert!(!loaded.contains_key("issues/2"));
}

#[test]
fn save_empty_state_preserves_existing_subtrees() {
    // Subtrees not mentioned in the new state at all are preserved — this is
    // documented behavior, not a bug. Saving {} is effectively a no-op.
    let (_dir, repo) = test_repo();

    let mut state = HashMap::new();
    state.insert("issues/1".to_string(), "a".repeat(40));
    save_sync_state(&repo, "org", "repo", &state).unwrap();

    save_sync_state(&repo, "org", "repo", &HashMap::new()).unwrap();

    let loaded = load_sync_state(&repo, "org", "repo").unwrap();
    assert!(
        loaded.contains_key("issues/1"),
        "unmentioned subtrees are preserved"
    );
}

#[test]
fn save_state_different_owner_repo_pairs_are_isolated() {
    let (_dir, repo) = test_repo();

    let mut s1 = HashMap::new();
    s1.insert("issues/1".to_string(), "a".repeat(40));
    save_sync_state(&repo, "org-a", "repo-x", &s1).unwrap();

    let mut s2 = HashMap::new();
    s2.insert("issues/1".to_string(), "b".repeat(40));
    save_sync_state(&repo, "org-b", "repo-x", &s2).unwrap();

    let loaded_a = load_sync_state(&repo, "org-a", "repo-x").unwrap();
    let loaded_b = load_sync_state(&repo, "org-b", "repo-x").unwrap();

    assert_eq!(
        loaded_a.get("issues/1").map(String::as_str),
        Some("a".repeat(40).as_str())
    );
    assert_eq!(
        loaded_b.get("issues/1").map(String::as_str),
        Some("b".repeat(40).as_str())
    );
}

#[test]
fn save_state_ref_is_created_in_repo() {
    let (_dir, repo) = test_repo();

    let mut state = HashMap::new();
    state.insert("issues/1".to_string(), "a".repeat(40));
    save_sync_state(&repo, "org", "repo", &state).unwrap();

    let ref_name = sync_ref_name("org", "repo");
    assert!(repo.find_reference(&ref_name).is_ok());
}

// ---------------------------------------------------------------------------
// lookup_by_github_id
// ---------------------------------------------------------------------------

#[test]
fn lookup_by_github_id_found() {
    let mut state = HashMap::new();
    state.insert("issues/7".to_string(), "abc123".to_string());

    assert_eq!(lookup_by_github_id(&state, "issues", 7), Some("abc123"));
}

#[test]
fn lookup_by_github_id_not_found() {
    let state: HashMap<String, String> = HashMap::new();
    assert!(lookup_by_github_id(&state, "issues", 7).is_none());
}

#[test]
fn lookup_by_github_id_wrong_kind() {
    let mut state = HashMap::new();
    state.insert("reviews/7".to_string(), "abc123".to_string());

    // "issues/7" is not present, even though 7 exists under "reviews".
    assert!(lookup_by_github_id(&state, "issues", 7).is_none());
}

// ---------------------------------------------------------------------------
// lookup_by_forge_oid
// ---------------------------------------------------------------------------

#[test]
fn lookup_by_forge_oid_found() {
    let mut state = HashMap::new();
    state.insert("issues/42".to_string(), "myoid123".to_string());

    assert_eq!(lookup_by_forge_oid(&state, "issues", "myoid123"), Some(42));
}

#[test]
fn lookup_by_forge_oid_not_found() {
    let state: HashMap<String, String> = HashMap::new();
    assert!(lookup_by_forge_oid(&state, "issues", "ghost").is_none());
}

#[test]
fn lookup_by_forge_oid_wrong_kind() {
    let mut state = HashMap::new();
    state.insert("reviews/1".to_string(), "myoid".to_string());

    // The OID exists but under "reviews", not "issues".
    assert!(lookup_by_forge_oid(&state, "issues", "myoid").is_none());
}

#[test]
fn lookup_by_forge_oid_returns_correct_number() {
    let mut state = HashMap::new();
    state.insert("issues/1".to_string(), "oid-one".to_string());
    state.insert("issues/99".to_string(), "oid-ninetynine".to_string());

    assert_eq!(
        lookup_by_forge_oid(&state, "issues", "oid-ninetynine"),
        Some(99)
    );
}
