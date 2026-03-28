//! Integration tests for config blob I/O in `refs.rs`.
#![allow(clippy::must_use_candidate, clippy::missing_panics_doc, missing_docs)]

use git_forge::refs::{read_config_blob, read_config_subtree, write_config_blob};
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
// read_config_blob
// ---------------------------------------------------------------------------

#[test]
fn read_config_blob_nonexistent_ref_returns_none() {
    let (_dir, repo) = test_repo();
    let result = read_config_blob(&repo, "any/path").unwrap();
    assert!(result.is_none());
}

#[test]
fn read_config_blob_nonexistent_path_returns_none() {
    let (_dir, repo) = test_repo();
    write_config_blob(&repo, "a/b", "value").unwrap();

    // The ref exists now but this path does not.
    let result = read_config_blob(&repo, "a/missing").unwrap();
    assert!(result.is_none());
}

// ---------------------------------------------------------------------------
// write_config_blob / read_config_blob round-trip
// ---------------------------------------------------------------------------

#[test]
fn write_then_read_roundtrip() {
    let (_dir, repo) = test_repo();
    write_config_blob(&repo, "provider/github/owner/repo/key", "some-value").unwrap();

    let val = read_config_blob(&repo, "provider/github/owner/repo/key")
        .unwrap()
        .expect("value");
    assert_eq!(val, "some-value");
}

#[test]
fn write_overwrites_same_path() {
    let (_dir, repo) = test_repo();
    write_config_blob(&repo, "key", "first").unwrap();
    write_config_blob(&repo, "key", "second").unwrap();

    let val = read_config_blob(&repo, "key").unwrap().expect("value");
    assert_eq!(val, "second");
}

#[test]
fn write_multiple_paths_each_preserved() {
    let (_dir, repo) = test_repo();
    write_config_blob(&repo, "a/x", "alpha").unwrap();
    write_config_blob(&repo, "a/y", "beta").unwrap();
    write_config_blob(&repo, "b/z", "gamma").unwrap();

    assert_eq!(
        read_config_blob(&repo, "a/x").unwrap().as_deref(),
        Some("alpha")
    );
    assert_eq!(
        read_config_blob(&repo, "a/y").unwrap().as_deref(),
        Some("beta")
    );
    assert_eq!(
        read_config_blob(&repo, "b/z").unwrap().as_deref(),
        Some("gamma")
    );
}

#[test]
fn write_deeply_nested_path() {
    let (_dir, repo) = test_repo();
    write_config_blob(&repo, "provider/github/my-org/my-repo/sigil/issue", "GH#").unwrap();

    let val = read_config_blob(&repo, "provider/github/my-org/my-repo/sigil/issue")
        .unwrap()
        .expect("value");
    assert_eq!(val, "GH#");
}

#[test]
fn write_config_blob_empty_value() {
    let (_dir, repo) = test_repo();
    write_config_blob(&repo, "key", "").unwrap();

    let val = read_config_blob(&repo, "key").unwrap().expect("value");
    assert_eq!(val, "");
}

#[test]
fn write_config_blob_unicode_value() {
    let (_dir, repo) = test_repo();
    write_config_blob(&repo, "msg", "日本語").unwrap();

    let val = read_config_blob(&repo, "msg").unwrap().expect("value");
    assert_eq!(val, "日本語");
}

#[test]
fn write_sibling_paths_do_not_clobber_each_other() {
    let (_dir, repo) = test_repo();
    // Write two siblings under the same parent.
    write_config_blob(&repo, "provider/github/org/repo-a/sigil/issue", "A#").unwrap();
    write_config_blob(&repo, "provider/github/org/repo-b/sigil/issue", "B#").unwrap();

    assert_eq!(
        read_config_blob(&repo, "provider/github/org/repo-a/sigil/issue")
            .unwrap()
            .as_deref(),
        Some("A#")
    );
    assert_eq!(
        read_config_blob(&repo, "provider/github/org/repo-b/sigil/issue")
            .unwrap()
            .as_deref(),
        Some("B#")
    );
}

// ---------------------------------------------------------------------------
// read_config_subtree
// ---------------------------------------------------------------------------

#[test]
fn read_config_subtree_nonexistent_ref_returns_empty() {
    let (_dir, repo) = test_repo();
    let map = read_config_subtree(&repo, "provider/github/org/repo/sigil").unwrap();
    assert!(map.is_empty());
}

#[test]
fn read_config_subtree_nonexistent_path_returns_empty() {
    let (_dir, repo) = test_repo();
    write_config_blob(&repo, "other/key", "val").unwrap();

    let map = read_config_subtree(&repo, "provider/github/org/repo/sigil").unwrap();
    assert!(map.is_empty());
}

#[test]
fn read_config_subtree_returns_all_entries() {
    let (_dir, repo) = test_repo();
    write_config_blob(&repo, "sigils/issue", "GH#").unwrap();
    write_config_blob(&repo, "sigils/review", "PR#").unwrap();

    let map = read_config_subtree(&repo, "sigils").unwrap();
    assert_eq!(map.len(), 2);
    assert_eq!(map.get("issue").map(String::as_str), Some("GH#"));
    assert_eq!(map.get("review").map(String::as_str), Some("PR#"));
}

#[test]
fn read_config_subtree_does_not_include_other_subtrees() {
    let (_dir, repo) = test_repo();
    write_config_blob(&repo, "sigils/issue", "GH#").unwrap();
    write_config_blob(&repo, "other/thing", "val").unwrap();

    let map = read_config_subtree(&repo, "sigils").unwrap();
    assert_eq!(map.len(), 1);
    assert!(map.contains_key("issue"));
    assert!(!map.contains_key("thing"));
}
