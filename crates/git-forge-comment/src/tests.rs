use git2::Repository;
use tempfile::TempDir;

use crate::git2::{blob_oid_for_path, parse_ranges};
use crate::{Anchor, Comments, COMMENTS_REF_PREFIX};

// ---------------------------------------------------------------------------
// Fixtures
// ---------------------------------------------------------------------------

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

/// Repo with a single file `hello.txt` committed to HEAD.
fn repo_with_file() -> (TempDir, Repository) {
    let dir = TempDir::new().unwrap();
    let repo = Repository::init(dir.path()).unwrap();
    {
        let sig = repo.signature().unwrap();
        let blob_oid = repo.blob(b"hello\nworld\n").unwrap();
        let mut tb = repo.treebuilder(None).unwrap();
        tb.insert("hello.txt", blob_oid, 0o100_644).unwrap();
        let tree_oid = tb.write().unwrap();
        let tree = repo.find_tree(tree_oid).unwrap();
        repo.commit(Some("HEAD"), &sig, &sig, "add hello.txt", &tree, &[])
            .unwrap();
    }
    (dir, repo)
}

fn dummy_anchor(repo: &Repository) -> Anchor {
    let oid = repo.head().unwrap().peel_to_commit().unwrap().id();
    Anchor::Commit(oid)
}

fn ref_name() -> String {
    format!("{COMMENTS_REF_PREFIX}issue/1")
}

// ---------------------------------------------------------------------------
// parse_ranges
// ---------------------------------------------------------------------------

#[test]
fn parse_ranges_single() {
    assert_eq!(parse_ranges("1-5"), vec![(1, 5)]);
}

#[test]
fn parse_ranges_union() {
    assert_eq!(parse_ranges("1-5,10-15,20-30"), vec![(1, 5), (10, 15), (20, 30)]);
}

#[test]
fn parse_ranges_empty_string() {
    assert_eq!(parse_ranges(""), Vec::<(u32, u32)>::new());
}

#[test]
fn parse_ranges_ignores_malformed() {
    // "abc" has no '-' separator so it is silently dropped
    assert_eq!(parse_ranges("1-5,abc,10-15"), vec![(1, 5), (10, 15)]);
}

// ---------------------------------------------------------------------------
// blob_oid_for_path
// ---------------------------------------------------------------------------

#[test]
fn blob_oid_for_path_resolves_known_file() {
    let (_dir, repo) = repo_with_file();
    let oid = blob_oid_for_path(&repo, "hello.txt").unwrap();
    let expected = repo.blob(b"hello\nworld\n").unwrap();
    assert_eq!(oid, expected);
}

#[test]
fn blob_oid_for_path_errors_on_missing_path() {
    let (_dir, repo) = repo_with_file();
    assert!(blob_oid_for_path(&repo, "nonexistent.txt").is_err());
}

// ---------------------------------------------------------------------------
// Comments trait — basic lifecycle
// ---------------------------------------------------------------------------

#[test]
fn add_comment_creates_ref() {
    let (_dir, repo) = repo();
    let rn = ref_name();
    let anchor = dummy_anchor(&repo);
    let oid = repo.add_comment(&rn, &anchor, "hello").unwrap();
    let tip = repo.find_reference(&rn).unwrap().peel_to_commit().unwrap();
    assert_eq!(tip.id(), oid);
}

#[test]
fn comments_on_empty_ref_returns_empty_vec() {
    let (_dir, repo) = repo();
    let comments = repo.comments_on(&ref_name()).unwrap();
    assert!(comments.is_empty());
}

#[test]
fn comments_on_returns_in_order() {
    let (_dir, repo) = repo();
    let rn = ref_name();
    let anchor = dummy_anchor(&repo);
    repo.add_comment(&rn, &anchor, "first").unwrap();
    repo.add_comment(&rn, &anchor, "second").unwrap();
    repo.add_comment(&rn, &anchor, "third").unwrap();
    let comments = repo.comments_on(&rn).unwrap();
    assert_eq!(comments.len(), 3);
    // reverse-chronological order (tip first)
    assert_eq!(comments[0].body, "third");
    assert_eq!(comments[1].body, "second");
    assert_eq!(comments[2].body, "first");
}

#[test]
fn reply_sets_second_parent() {
    let (_dir, repo) = repo();
    let rn = ref_name();
    let anchor = dummy_anchor(&repo);
    let comment_oid = repo.add_comment(&rn, &anchor, "original").unwrap();
    let reply_oid = repo.reply_to_comment(&rn, comment_oid, "reply").unwrap();
    let reply_commit = repo.find_commit(reply_oid).unwrap();
    assert_eq!(reply_commit.parent_count(), 2);
    assert_eq!(reply_commit.parent_id(1).unwrap(), comment_oid);
}

#[test]
fn resolve_sets_resolved_trailer() {
    let (_dir, repo) = repo();
    let rn = ref_name();
    let anchor = dummy_anchor(&repo);
    let comment_oid = repo.add_comment(&rn, &anchor, "needs resolution").unwrap();
    repo.resolve_comment(&rn, comment_oid).unwrap();
    let comments = repo.comments_on(&rn).unwrap();
    let resolution = comments.iter().find(|c| c.resolved).unwrap();
    assert!(resolution.resolved);
}

#[test]
fn edit_creates_new_comment_with_replaces_trailer() {
    let (_dir, repo) = repo();
    let rn = ref_name();
    let anchor = dummy_anchor(&repo);
    let original_oid = repo.add_comment(&rn, &anchor, "original body").unwrap();
    let edit_oid = repo.edit_comment(&rn, original_oid, "new body").unwrap();
    let edit = repo.find_comment(&rn, edit_oid).unwrap().unwrap();
    assert_eq!(edit.body, "new body");
    assert_eq!(edit.replaces_oid, Some(original_oid));
    // original is still there
    let original = repo.find_comment(&rn, original_oid).unwrap().unwrap();
    assert_eq!(original.body, "original body");
}

#[test]
fn find_comment_returns_none_for_missing() {
    let (_dir, repo) = repo();
    let rn = ref_name();
    let anchor = dummy_anchor(&repo);
    repo.add_comment(&rn, &anchor, "exists").unwrap();
    let random_oid = git2::Oid::from_str("0000000000000000000000000000000000000001").unwrap();
    let result = repo.find_comment(&rn, random_oid).unwrap();
    assert!(result.is_none());
}

// ---------------------------------------------------------------------------
// Blob anchors — type inference and line ranges
// ---------------------------------------------------------------------------

#[test]
fn blob_anchor_roundtrip_no_ranges() {
    let (_dir, repo) = repo_with_file();
    let rn = ref_name();
    let blob_oid = blob_oid_for_path(&repo, "hello.txt").unwrap();
    let anchor = Anchor::Blob { oid: blob_oid, line_ranges: vec![] };
    let oid = repo.add_comment(&rn, &anchor, "whole file").unwrap();
    let comment = repo.find_comment(&rn, oid).unwrap().unwrap();
    match comment.anchor {
        Anchor::Blob { oid: got_oid, line_ranges } => {
            assert_eq!(got_oid, blob_oid);
            assert!(line_ranges.is_empty());
        }
        other => panic!("expected Blob, got {other:?}"),
    }
}

#[test]
fn blob_anchor_roundtrip_single_range() {
    let (_dir, repo) = repo_with_file();
    let rn = ref_name();
    let blob_oid = blob_oid_for_path(&repo, "hello.txt").unwrap();
    let anchor = Anchor::Blob { oid: blob_oid, line_ranges: vec![(1, 5)] };
    let oid = repo.add_comment(&rn, &anchor, "first five lines").unwrap();
    let comment = repo.find_comment(&rn, oid).unwrap().unwrap();
    match comment.anchor {
        Anchor::Blob { line_ranges, .. } => assert_eq!(line_ranges, vec![(1, 5)]),
        other => panic!("expected Blob, got {other:?}"),
    }
}

#[test]
fn blob_anchor_roundtrip_union_ranges() {
    let (_dir, repo) = repo_with_file();
    let rn = ref_name();
    let blob_oid = blob_oid_for_path(&repo, "hello.txt").unwrap();
    let ranges = vec![(1, 5), (10, 15), (20, 30)];
    let anchor = Anchor::Blob { oid: blob_oid, line_ranges: ranges.clone() };
    let oid = repo.add_comment(&rn, &anchor, "union ranges").unwrap();
    let comment = repo.find_comment(&rn, oid).unwrap().unwrap();
    match comment.anchor {
        Anchor::Blob { line_ranges, .. } => assert_eq!(line_ranges, ranges),
        other => panic!("expected Blob, got {other:?}"),
    }
}

#[test]
fn anchor_type_inferred_as_blob_from_oid() {
    let (_dir, repo) = repo_with_file();
    let rn = ref_name();
    let blob_oid = blob_oid_for_path(&repo, "hello.txt").unwrap();
    // Use Blob anchor directly — the serialized form stores the OID;
    // on read the object kind determines the variant.
    let anchor = Anchor::Blob { oid: blob_oid, line_ranges: vec![] };
    let oid = repo.add_comment(&rn, &anchor, "inferred blob").unwrap();
    let comment = repo.find_comment(&rn, oid).unwrap().unwrap();
    assert!(matches!(comment.anchor, Anchor::Blob { .. }));
}

#[test]
fn anchor_type_inferred_as_tree_from_oid() {
    let (_dir, repo) = repo_with_file();
    let rn = ref_name();
    let tree_oid = repo.head().unwrap().peel_to_tree().unwrap().id();
    let anchor = Anchor::Tree(tree_oid);
    let oid = repo.add_comment(&rn, &anchor, "tree anchor").unwrap();
    let comment = repo.find_comment(&rn, oid).unwrap().unwrap();
    assert!(matches!(comment.anchor, Anchor::Tree(_)));
}

#[test]
fn anchor_type_inferred_as_commit_from_oid() {
    let (_dir, repo) = repo_with_file();
    let rn = ref_name();
    let commit_oid = repo.head().unwrap().peel_to_commit().unwrap().id();
    let anchor = Anchor::Commit(commit_oid);
    let oid = repo.add_comment(&rn, &anchor, "commit anchor").unwrap();
    let comment = repo.find_comment(&rn, oid).unwrap().unwrap();
    assert!(matches!(comment.anchor, Anchor::Commit(_)));
}

// ---------------------------------------------------------------------------
// build_anchor (exe logic)
// ---------------------------------------------------------------------------

#[test]
fn build_anchor_defaults_to_head_commit() {
    let (_dir, repo) = repo();
    let anchor = crate::exe::build_anchor(&repo, None, None, None).unwrap();
    let head_oid = repo.head().unwrap().peel_to_commit().unwrap().id();
    assert!(matches!(anchor, Anchor::Commit(oid) if oid == head_oid));
}

#[test]
fn build_anchor_infers_blob_from_oid() {
    let (_dir, repo) = repo_with_file();
    let blob_oid = blob_oid_for_path(&repo, "hello.txt").unwrap();
    let oid_str = blob_oid.to_string();
    let anchor = crate::exe::build_anchor(&repo, Some(&oid_str), None, None).unwrap();
    assert!(matches!(anchor, Anchor::Blob { .. }));
}

#[test]
fn build_anchor_infers_tree_from_oid() {
    let (_dir, repo) = repo_with_file();
    let tree_oid = repo.head().unwrap().peel_to_tree().unwrap().id();
    let oid_str = tree_oid.to_string();
    let anchor = crate::exe::build_anchor(&repo, Some(&oid_str), None, None).unwrap();
    assert!(matches!(anchor, Anchor::Tree(_)));
}

#[test]
fn build_anchor_path_resolves_to_blob() {
    let (_dir, repo) = repo_with_file();
    let blob_oid = blob_oid_for_path(&repo, "hello.txt").unwrap();
    let anchor =
        crate::exe::build_anchor(&repo, Some("hello.txt"), None, None).unwrap();
    match anchor {
        Anchor::Blob { oid, line_ranges } => {
            assert_eq!(oid, blob_oid);
            assert!(line_ranges.is_empty());
        }
        other => panic!("expected Blob, got {other:?}"),
    }
}

#[test]
fn build_anchor_path_with_union_ranges() {
    let (_dir, repo) = repo_with_file();
    let anchor = crate::exe::build_anchor(
        &repo,
        Some("hello.txt"),
        None,
        Some("1-3,5-7"),
    )
    .unwrap();
    match anchor {
        Anchor::Blob { line_ranges, .. } => {
            assert_eq!(line_ranges, vec![(1, 3), (5, 7)]);
        }
        other => panic!("expected Blob, got {other:?}"),
    }
}

#[test]
fn build_anchor_explicit_blob_type_with_union_ranges() {
    let (_dir, repo) = repo_with_file();
    let blob_oid = blob_oid_for_path(&repo, "hello.txt").unwrap();
    let oid_str = blob_oid.to_string();
    let anchor = crate::exe::build_anchor(
        &repo,
        Some(&oid_str),
        Some("blob"),
        Some("1-5,10-15"),
    )
    .unwrap();
    match anchor {
        Anchor::Blob { line_ranges, .. } => {
            assert_eq!(line_ranges, vec![(1, 5), (10, 15)]);
        }
        other => panic!("expected Blob, got {other:?}"),
    }
}

#[test]
fn build_anchor_unknown_type_errors() {
    let (_dir, repo) = repo();
    let commit_oid = repo.head().unwrap().peel_to_commit().unwrap().id();
    let oid_str = commit_oid.to_string();
    let result = crate::exe::build_anchor(
        &repo,
        Some(&oid_str),
        Some("bogus"),
        None,
    );
    assert!(result.is_err());
}

#[test]
fn build_anchor_bad_oid_and_bad_path_errors() {
    let (_dir, repo) = repo();
    let result = crate::exe::build_anchor(&repo, Some("not-an-oid-or-path"), None, None);
    assert!(result.is_err());
}

#[test]
fn edit_chain_replaces_points_at_previous_edit() {
    let (_dir, repo) = repo();
    let rn = ref_name();
    let anchor = dummy_anchor(&repo);
    let original_oid = repo.add_comment(&rn, &anchor, "v1").unwrap();
    let edit1_oid = repo.edit_comment(&rn, original_oid, "v2").unwrap();
    let edit2_oid = repo.edit_comment(&rn, edit1_oid, "v3").unwrap();
    let edit2 = repo.find_comment(&rn, edit2_oid).unwrap().unwrap();
    assert_eq!(edit2.replaces_oid, Some(edit1_oid));
    assert_eq!(edit2.body, "v3");
}

#[test]
fn build_anchor_malformed_range_errors() {
    let (_dir, repo) = repo_with_file();
    let result = crate::exe::build_anchor(
        &repo,
        Some("hello.txt"),
        None,
        Some("abc"),
    );
    assert!(result.is_err());
}

#[test]
fn build_anchor_inverted_range_errors() {
    let (_dir, repo) = repo_with_file();
    let result = crate::exe::build_anchor(
        &repo,
        Some("hello.txt"),
        None,
        Some("5-1"),
    );
    assert!(result.is_err());
}

#[test]
fn build_anchor_zero_based_range_errors() {
    let (_dir, repo) = repo_with_file();
    let result = crate::exe::build_anchor(
        &repo,
        Some("hello.txt"),
        None,
        Some("0-5"),
    );
    assert!(result.is_err());
}
