//! Integration tests for the git-forge-comment crate.

use git2::Repository;
use tempfile::TempDir;

use git_forge_comment::git2::blob_oid_for_path;
use git_forge_comment::{Anchor, Comments, COMMENTS_REF_PREFIX};

// ---------------------------------------------------------------------------
// Fixtures
// ---------------------------------------------------------------------------

fn repo_with_file() -> (TempDir, Repository) {
    let dir = TempDir::new().unwrap();
    let repo = Repository::init(dir.path()).unwrap();
    {
        let sig = repo.signature().unwrap();
        let blob_oid = repo.blob(b"line1\nline2\nline3\nline4\nline5\n").unwrap();
        let mut tb = repo.treebuilder(None).unwrap();
        tb.insert("src.txt", blob_oid, 0o100644).unwrap();
        let tree_oid = tb.write().unwrap();
        let tree = repo.find_tree(tree_oid).unwrap();
        repo.commit(Some("HEAD"), &sig, &sig, "initial", &tree, &[]).unwrap();
    }
    (dir, repo)
}

fn comments_ref() -> String {
    format!("{COMMENTS_REF_PREFIX}review/1")
}

// ---------------------------------------------------------------------------
// Full comment lifecycle
// ---------------------------------------------------------------------------

#[test]
fn lifecycle_add_reply_edit_resolve() {
    let (_dir, repo) = repo_with_file();
    let rn = comments_ref();
    let commit_oid = repo.head().unwrap().peel_to_commit().unwrap().id();
    let anchor = Anchor::Commit(commit_oid);

    // Add
    let c1 = repo.add_comment(&rn, &anchor, "initial comment").unwrap();
    // Reply
    let r1 = repo.reply_to_comment(&rn, c1, "a reply").unwrap();
    // Edit the original
    let e1 = repo.edit_comment(&rn, c1, "edited comment").unwrap();
    // Resolve
    let _res = repo.resolve_comment(&rn, c1).unwrap();

    let all = repo.comments_on(&rn).unwrap();
    assert_eq!(all.len(), 4);

    let reply = repo.find_comment(&rn, r1).unwrap().unwrap();
    assert_eq!(reply.parent_oid, Some(c1));

    let edit = repo.find_comment(&rn, e1).unwrap().unwrap();
    assert_eq!(edit.replaces_oid, Some(c1));
    assert_eq!(edit.body, "edited comment");

    let resolved = all.iter().find(|c| c.resolved).unwrap();
    assert!(resolved.resolved);
}

// ---------------------------------------------------------------------------
// Path anchor: resolve a file path to its blob OID
// ---------------------------------------------------------------------------

#[test]
fn path_anchor_roundtrip() {
    let (_dir, repo) = repo_with_file();
    let rn = comments_ref();

    let blob_oid = blob_oid_for_path(&repo, "src.txt").unwrap();
    let anchor = Anchor::Blob { oid: blob_oid, line_ranges: vec![] };

    let oid = repo.add_comment(&rn, &anchor, "whole file comment").unwrap();
    let comment = repo.find_comment(&rn, oid).unwrap().unwrap();

    match comment.anchor {
        Anchor::Blob { oid: got, line_ranges } => {
            assert_eq!(got, blob_oid);
            assert!(line_ranges.is_empty());
        }
        other => panic!("expected Blob, got {other:?}"),
    }
}

// ---------------------------------------------------------------------------
// Union line ranges
// ---------------------------------------------------------------------------

#[test]
fn union_line_ranges_stored_and_retrieved() {
    let (_dir, repo) = repo_with_file();
    let rn = comments_ref();

    let blob_oid = blob_oid_for_path(&repo, "src.txt").unwrap();
    let ranges = vec![(1, 2), (4, 5)];
    let anchor = Anchor::Blob { oid: blob_oid, line_ranges: ranges.clone() };

    let oid = repo.add_comment(&rn, &anchor, "multi-range comment").unwrap();
    let comment = repo.find_comment(&rn, oid).unwrap().unwrap();

    match comment.anchor {
        Anchor::Blob { line_ranges, .. } => assert_eq!(line_ranges, ranges),
        other => panic!("expected Blob, got {other:?}"),
    }
}

// ---------------------------------------------------------------------------
// build_anchor: path resolution and type inference
// ---------------------------------------------------------------------------

#[test]
fn build_anchor_path_infers_blob() {
    let (_dir, repo) = repo_with_file();
    let expected_oid = blob_oid_for_path(&repo, "src.txt").unwrap();

    let anchor =
        git_forge_comment::exe::build_anchor(&repo, Some("src.txt".to_string()), None, None)
            .unwrap();

    match anchor {
        Anchor::Blob { oid, line_ranges } => {
            assert_eq!(oid, expected_oid);
            assert!(line_ranges.is_empty());
        }
        other => panic!("expected Blob, got {other:?}"),
    }
}

#[test]
fn build_anchor_path_with_ranges() {
    let (_dir, repo) = repo_with_file();

    let anchor = git_forge_comment::exe::build_anchor(
        &repo,
        Some("src.txt".to_string()),
        None,
        Some("1-2,4-5".to_string()),
    )
    .unwrap();

    match anchor {
        Anchor::Blob { line_ranges, .. } => assert_eq!(line_ranges, vec![(1, 2), (4, 5)]),
        other => panic!("expected Blob, got {other:?}"),
    }
}

#[test]
fn build_anchor_oid_infers_commit() {
    let (_dir, repo) = repo_with_file();
    let commit_oid = repo.head().unwrap().peel_to_commit().unwrap().id();

    let anchor =
        git_forge_comment::exe::build_anchor(&repo, Some(commit_oid.to_string()), None, None)
            .unwrap();

    assert!(matches!(anchor, Anchor::Commit(_)));
}

#[test]
fn build_anchor_oid_infers_blob() {
    let (_dir, repo) = repo_with_file();
    let blob_oid = blob_oid_for_path(&repo, "src.txt").unwrap();

    let anchor =
        git_forge_comment::exe::build_anchor(&repo, Some(blob_oid.to_string()), None, None)
            .unwrap();

    assert!(matches!(anchor, Anchor::Blob { .. }));
}

#[test]
fn build_anchor_oid_infers_tree() {
    let (_dir, repo) = repo_with_file();
    let tree_oid = repo.head().unwrap().peel_to_tree().unwrap().id();

    let anchor =
        git_forge_comment::exe::build_anchor(&repo, Some(tree_oid.to_string()), None, None)
            .unwrap();

    assert!(matches!(anchor, Anchor::Tree(_)));
}
