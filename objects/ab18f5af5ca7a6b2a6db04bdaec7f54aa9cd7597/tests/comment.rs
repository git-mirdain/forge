//! Integration tests for comment chains.
#![allow(clippy::must_use_candidate, clippy::missing_panics_doc, missing_docs)]

use git2::Repository;
use tempfile::TempDir;

use git_forge::Store;
use git_forge::comment::list_thread;
use git_forge::comment::{
    Anchor, add_comment, add_reply, edit_comment, issue_comment_ref, list_comments, resolve_comment,
};
use git_forge::exe::Executor;

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
        oid: "asdfhjkl".to_string(),
        path: None,
        range: Some("10-20".to_string()),
    };
    let comment = add_comment(&repo, &ref_name, "line comment", Some(&anchor)).unwrap();

    let a = comment.anchor.as_ref().unwrap();
    match a {
        Anchor::Object { oid, range, .. } => {
            assert_eq!(oid, "asdfhjkl");
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

#[test]
fn body_with_trailer_like_text_survives_roundtrip() {
    let (_dir, repo) = test_repo();
    let ref_name = issue_comment_ref("abc123");
    let body = "The fix is: set Key: value in the config\nSigned-off-by: someone";
    let comment = add_comment(&repo, &ref_name, body, None).unwrap();

    let comments = list_comments(&repo, &ref_name).unwrap();
    assert_eq!(comments.len(), 1);
    assert_eq!(comments[0].body, body);
    assert_eq!(comments[0].oid, comment.oid);
}

#[test]
fn list_comments_empty_chain() {
    let (_dir, repo) = test_repo();
    let ref_name = issue_comment_ref("nonexistent");
    let comments = list_comments(&repo, &ref_name).unwrap();
    assert!(comments.is_empty());
}

#[test]
fn resolve_comment_with_message() {
    let (_dir, repo) = test_repo();
    let ref_name = issue_comment_ref("abc123");
    let root = add_comment(&repo, &ref_name, "needs work", None).unwrap();
    let resolution = resolve_comment(
        &repo,
        &ref_name,
        &root.oid,
        Some("addressed in latest push"),
    )
    .unwrap();

    assert!(resolution.resolved);
    assert_eq!(resolution.body, "addressed in latest push");
    assert_eq!(resolution.reply_to.as_deref(), Some(root.oid.as_str()));
}

#[test]
fn deep_thread_reply_to_reply() {
    let (_dir, repo) = test_repo();
    let ref_name = issue_comment_ref("abc123");
    let root = add_comment(&repo, &ref_name, "root", None).unwrap();
    let child = add_reply(&repo, &ref_name, "child", &root.oid, None).unwrap();
    let grandchild = add_reply(&repo, &ref_name, "grandchild", &child.oid, None).unwrap();
    // unrelated top-level comment should not appear in the thread
    add_comment(&repo, &ref_name, "unrelated", None).unwrap();

    let thread = git_forge::comment::list_thread(&repo, &ref_name, &root.oid).unwrap();
    let oids: Vec<&str> = thread.iter().map(|c| c.oid.as_str()).collect();
    assert_eq!(thread.len(), 3);
    assert!(oids.contains(&root.oid.as_str()));
    assert!(oids.contains(&child.oid.as_str()));
    assert!(oids.contains(&grandchild.oid.as_str()));
    assert_eq!(grandchild.reply_to.as_deref(), Some(child.oid.as_str()));
}

#[test]
fn executor_add_and_list_comments() {
    let (_dir, repo) = test_repo();
    let store = Store::new(&repo);
    let issue = store.create_issue("Test issue", "body", &[], &[]).unwrap();
    let exec = Executor::from_path(repo.path().parent().unwrap()).unwrap();

    let c1 = exec.add_issue_comment(&issue.oid, "first", None).unwrap();
    let c2 = exec.add_issue_comment(&issue.oid, "second", None).unwrap();

    let comments = exec.list_issue_comments(&issue.oid).unwrap();
    assert_eq!(comments.len(), 2);
    let oids: Vec<&str> = comments.iter().map(|c| c.oid.as_str()).collect();
    assert!(oids.contains(&c1.oid.as_str()));
    assert!(oids.contains(&c2.oid.as_str()));
}

#[test]
fn executor_reply_and_resolve() {
    let (_dir, repo) = test_repo();
    let store = Store::new(&repo);
    let issue = store.create_issue("Test issue", "body", &[], &[]).unwrap();
    let exec = Executor::from_path(repo.path().parent().unwrap()).unwrap();

    let root = exec.add_issue_comment(&issue.oid, "root", None).unwrap();
    let reply = exec
        .reply_issue_comment(&issue.oid, "reply text", &root.oid, None)
        .unwrap();
    assert_eq!(reply.reply_to.as_deref(), Some(root.oid.as_str()));

    let resolved = exec
        .resolve_issue_comment(&issue.oid, &root.oid, Some("done"))
        .unwrap();
    assert!(resolved.resolved);
    assert_eq!(resolved.body, "done");
}

#[test]
fn executor_comment_on_nonexistent_issue() {
    let (_dir, repo) = test_repo();
    let exec = Executor::from_path(repo.path().parent().unwrap()).unwrap();

    let result = exec.add_issue_comment("nonexistent", "body", None);
    assert!(result.is_err());
}

#[test]
fn edit_preserves_anchor() {
    let (_dir, repo) = test_repo();
    let ref_name = issue_comment_ref("abc123");
    let anchor = Anchor::Object {
        oid: "asdfhjkl".to_string(),
        path: None,
        range: Some("5-10".to_string()),
    };
    let original = add_comment(&repo, &ref_name, "original", Some(&anchor)).unwrap();
    let edited = edit_comment(&repo, &ref_name, &original.oid, "edited", Some(&anchor)).unwrap();

    assert_eq!(edited.body, "edited");
    assert_eq!(edited.replaces.as_deref(), Some(original.oid.as_str()));
    match edited.anchor.as_ref().unwrap() {
        Anchor::Object { oid, range, .. } => {
            assert_eq!(oid, "asdfhjkl");
            assert_eq!(range.as_deref(), Some("5-10"));
        }
        Anchor::CommitRange { .. } => panic!("expected Object anchor"),
    }
}

// --- OID prefix resolution tests ---

#[test]
fn reply_accepts_oid_prefix() {
    let (_dir, repo) = test_repo();
    let ref_name = issue_comment_ref("abc123");
    let root = add_comment(&repo, &ref_name, "root", None).unwrap();

    let prefix = &root.oid[..8];
    let reply = add_reply(&repo, &ref_name, "prefix reply", prefix, None).unwrap();
    assert_eq!(reply.reply_to.as_deref(), Some(root.oid.as_str()));
}

#[test]
fn resolve_accepts_oid_prefix() {
    let (_dir, repo) = test_repo();
    let ref_name = issue_comment_ref("abc123");
    let root = add_comment(&repo, &ref_name, "root", None).unwrap();

    let prefix = &root.oid[..8];
    let resolution = resolve_comment(&repo, &ref_name, prefix, Some("done")).unwrap();
    assert!(resolution.resolved);
    assert_eq!(resolution.reply_to.as_deref(), Some(root.oid.as_str()));
}

#[test]
fn edit_accepts_oid_prefix() {
    let (_dir, repo) = test_repo();
    let ref_name = issue_comment_ref("abc123");
    let original = add_comment(&repo, &ref_name, "original", None).unwrap();

    let prefix = &original.oid[..8];
    let edited = edit_comment(&repo, &ref_name, prefix, "edited", None).unwrap();
    assert_eq!(edited.body, "edited");
    assert_eq!(edited.replaces.as_deref(), Some(original.oid.as_str()));
}

#[test]
fn list_thread_accepts_oid_prefix() {
    let (_dir, repo) = test_repo();
    let ref_name = issue_comment_ref("abc123");
    let root = add_comment(&repo, &ref_name, "root", None).unwrap();
    add_reply(&repo, &ref_name, "reply", &root.oid, None).unwrap();
    add_comment(&repo, &ref_name, "unrelated", None).unwrap();

    let prefix = &root.oid[..8];
    let thread = list_thread(&repo, &ref_name, prefix).unwrap();
    assert_eq!(thread.len(), 2);
}

#[test]
fn oid_prefix_not_found_errors() {
    let (_dir, repo) = test_repo();
    let ref_name = issue_comment_ref("abc123");
    add_comment(&repo, &ref_name, "root", None).unwrap();

    let result = add_reply(&repo, &ref_name, "reply", "asdfhjkl", None);
    assert!(result.is_err());
}

// --- Anchor path roundtrip (issue 1 regression) ---

#[test]
fn anchor_object_with_path_roundtrips() {
    let (_dir, repo) = test_repo();
    let ref_name = issue_comment_ref("abc123");
    let anchor = Anchor::Object {
        oid: "abc123".to_string(),
        path: Some("src/main.rs".to_string()),
        range: Some("42-47".to_string()),
    };
    let comment = add_comment(&repo, &ref_name, "path comment", Some(&anchor)).unwrap();

    match comment.anchor.as_ref().unwrap() {
        Anchor::Object { oid, path, range } => {
            assert_eq!(oid, "abc123");
            assert_eq!(path.as_deref(), Some("src/main.rs"));
            assert_eq!(range.as_deref(), Some("42-47"));
        }
        Anchor::CommitRange { .. } => panic!("expected Object anchor"),
    }

    // Also verify via list_comments roundtrip.
    let all = list_comments(&repo, &ref_name).unwrap();
    match all[0].anchor.as_ref().unwrap() {
        Anchor::Object { path, .. } => assert_eq!(path.as_deref(), Some("src/main.rs")),
        Anchor::CommitRange { .. } => panic!("expected Object anchor"),
    }
}
