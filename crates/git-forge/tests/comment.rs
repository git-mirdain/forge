//! Integration tests for comment chains.
#![allow(clippy::must_use_candidate, clippy::missing_panics_doc, missing_docs)]

use git2::Repository;
use tempfile::TempDir;

use git_forge::comment::{
    Anchor, add_comment, add_reply, edit_comment, issue_comment_ref, list_comments, resolve_comment,
};

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

#[test]
fn add_comment_creates_chain() {
    let (_dir, repo) = test_repo();
    let ref_name = issue_comment_ref("abc123");
    let comment = add_comment(&repo, &ref_name, "first comment", None).unwrap();

    assert_eq!(comment.body, "first comment");
    assert_eq!(comment.author_name, "test");
    assert_eq!(comment.author_email, "test@test.com");
    assert!(!comment.resolved);
    assert!(comment.replaces.is_none());
    assert!(comment.reply_to.is_none());
    assert!(comment.anchor.is_none());
    assert_eq!(comment.oid.len(), 40);
}

#[test]
fn add_second_comment() {
    let (_dir, repo) = test_repo();
    let ref_name = issue_comment_ref("abc123");
    let c1 = add_comment(&repo, &ref_name, "first", None).unwrap();
    let c2 = add_comment(&repo, &ref_name, "second", None).unwrap();

    assert_ne!(c1.oid, c2.oid);
    assert_eq!(c2.body, "second");
    assert!(c2.reply_to.is_none());
}

#[test]
fn reply_threading() {
    let (_dir, repo) = test_repo();
    let ref_name = issue_comment_ref("abc123");
    let root = add_comment(&repo, &ref_name, "root comment", None).unwrap();
    let reply = add_reply(&repo, &ref_name, "reply text", &root.oid, None).unwrap();

    assert_eq!(reply.reply_to.as_deref(), Some(root.oid.as_str()));
    assert_eq!(reply.body, "reply text");
}

#[test]
fn list_comments_chronological() {
    let (_dir, repo) = test_repo();
    let ref_name = issue_comment_ref("abc123");
    add_comment(&repo, &ref_name, "alpha", None).unwrap();
    add_comment(&repo, &ref_name, "beta", None).unwrap();
    add_comment(&repo, &ref_name, "gamma", None).unwrap();

    let comments = list_comments(&repo, &ref_name).unwrap();
    // walk returns tip-first (reverse chronological)
    assert_eq!(comments.len(), 3);
    assert_eq!(comments[0].body, "gamma");
    assert_eq!(comments[2].body, "alpha");
}

#[test]
fn list_comments_in_thread() {
    let (_dir, repo) = test_repo();
    let ref_name = issue_comment_ref("abc123");
    let root = add_comment(&repo, &ref_name, "root", None).unwrap();
    let reply = add_reply(&repo, &ref_name, "reply", &root.oid, None).unwrap();
    // unrelated top-level comment
    add_comment(&repo, &ref_name, "unrelated", None).unwrap();

    let thread = git_forge::comment::list_thread(&repo, &ref_name, &root.oid).unwrap();
    let oids: Vec<&str> = thread.iter().map(|c| c.oid.as_str()).collect();
    assert!(oids.contains(&root.oid.as_str()));
    assert!(oids.contains(&reply.oid.as_str()));
    assert_eq!(thread.len(), 2);
}

#[test]
fn resolve_sets_trailer() {
    let (_dir, repo) = test_repo();
    let ref_name = issue_comment_ref("abc123");
    let root = add_comment(&repo, &ref_name, "needs resolve", None).unwrap();
    let resolution = resolve_comment(&repo, &ref_name, &root.oid, None).unwrap();

    assert!(resolution.resolved);
    assert_eq!(resolution.reply_to.as_deref(), Some(root.oid.as_str()));
}

#[test]
fn edit_sets_replaces() {
    let (_dir, repo) = test_repo();
    let ref_name = issue_comment_ref("abc123");
    let original = add_comment(&repo, &ref_name, "original text", None).unwrap();
    let edited = edit_comment(&repo, &ref_name, &original.oid, "updated text", None).unwrap();

    assert_eq!(edited.body, "updated text");
    assert_eq!(edited.replaces.as_deref(), Some(original.oid.as_str()));
}

#[test]
fn anchor_object_with_range() {
    let (_dir, repo) = test_repo();
    let ref_name = issue_comment_ref("abc123");
    let anchor = Anchor::Object {
        oid: "deadbeef".to_string(),
        range: Some("10-20".to_string()),
    };
    let comment = add_comment(&repo, &ref_name, "line comment", Some(&anchor)).unwrap();

    let a = comment.anchor.as_ref().unwrap();
    match a {
        Anchor::Object { oid, range } => {
            assert_eq!(oid, "deadbeef");
            assert_eq!(range.as_deref(), Some("10-20"));
        }
        Anchor::CommitRange { .. } => panic!("expected Object anchor"),
    }
}

#[test]
fn anchor_commit_range() {
    let (_dir, repo) = test_repo();
    let ref_name = issue_comment_ref("abc123");
    let anchor = Anchor::CommitRange {
        start: "aaa".to_string(),
        end: "bbb".to_string(),
    };
    let comment = add_comment(&repo, &ref_name, "range comment", Some(&anchor)).unwrap();

    let a = comment.anchor.as_ref().unwrap();
    match a {
        Anchor::CommitRange { start, end } => {
            assert_eq!(start, "aaa");
            assert_eq!(end, "bbb");
        }
        Anchor::Object { .. } => panic!("expected CommitRange anchor"),
    }
}
