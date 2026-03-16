use git2::Repository;
use tempfile::TempDir;

use crate::{Anchor, Comments, COMMENTS_REF_PREFIX};

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

fn dummy_anchor(repo: &Repository) -> Anchor {
    let oid = repo.head().unwrap().peel_to_commit().unwrap().id();
    Anchor::Commit(oid)
}

fn ref_name() -> String {
    format!("{COMMENTS_REF_PREFIX}issue/1")
}

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
fn comments_on_returns_in_order() {
    let (_dir, repo) = repo();
    let rn = ref_name();
    let anchor = dummy_anchor(&repo);
    repo.add_comment(&rn, &anchor, "first").unwrap();
    repo.add_comment(&rn, &anchor, "second").unwrap();
    repo.add_comment(&rn, &anchor, "third").unwrap();
    let comments = repo.comments_on(&rn).unwrap();
    assert_eq!(comments.len(), 3);
    // comments_on returns in reverse-chronological order (tip first)
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
fn find_comment_returns_none_for_missing() {
    let (_dir, repo) = repo();
    let rn = ref_name();
    let anchor = dummy_anchor(&repo);
    repo.add_comment(&rn, &anchor, "exists").unwrap();
    let random_oid = git2::Oid::from_str("0000000000000000000000000000000000000001").unwrap();
    let result = repo.find_comment(&rn, random_oid).unwrap();
    assert!(result.is_none());
}
