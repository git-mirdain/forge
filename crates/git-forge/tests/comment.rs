//! Integration tests for comment chains (v2 API).
#![allow(clippy::must_use_candidate, clippy::missing_panics_doc, missing_docs)]

use git2::Repository;
use tempfile::TempDir;

use git_forge::Store;
use git_forge::comment::{
    Anchor, comment_thread_ref, create_thread, edit_in_thread, find_threads_by_object,
    index_lookup, list_thread_comments, rebuild_comments_index, reply_to_thread, resolve_thread,
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

// --- v2 core API ---

#[test]
fn create_thread_produces_ref() {
    let (_dir, repo) = test_repo();
    let blob_oid = repo.blob(b"anchor content").unwrap().to_string();
    let anchor = Anchor {
        oid: blob_oid,
        start_line: Some(10),
        end_line: Some(20),
    };
    let (thread_id, root) =
        create_thread(&repo, "first comment", Some(&anchor), None, None).unwrap();
    assert!(!thread_id.is_empty());
    assert_eq!(root.body, "first comment");

    let ref_name = comment_thread_ref(&thread_id);
    assert!(repo.find_reference(&ref_name).is_ok());
}

#[test]
fn thread_tree_roundtrip() {
    let (_dir, repo) = test_repo();
    let blob_oid = repo.blob(b"source file content").unwrap().to_string();
    let anchor = Anchor {
        oid: blob_oid.clone(),
        start_line: Some(1),
        end_line: None,
    };
    let (thread_id, _) = create_thread(&repo, "body text", Some(&anchor), None, None).unwrap();

    let comments = list_thread_comments(&repo, &thread_id).unwrap();
    assert_eq!(comments.len(), 1);
    assert_eq!(comments[0].body, "body text");
    // Anchor is set; the OID appears in the commit message trailer.
    assert!(comments[0].anchor.is_some());
    let commit_msg = repo
        .find_reference(&comment_thread_ref(&thread_id))
        .unwrap()
        .peel_to_commit()
        .unwrap()
        .message()
        .unwrap_or("")
        .to_string();
    assert!(
        commit_msg.contains(&blob_oid),
        "commit missing Anchor trailer"
    );
}

#[test]
fn reply_appends_to_chain() {
    let (_dir, repo) = test_repo();
    let (thread_id, root) = create_thread(&repo, "root", None, None, None).unwrap();

    let reply =
        reply_to_thread(&repo, &thread_id, "reply text", &root.oid, None, None, None).unwrap();
    assert_eq!(reply.reply_to.as_deref(), Some(root.oid.as_str()));

    let all = list_thread_comments(&repo, &thread_id).unwrap();
    assert_eq!(all.len(), 2);
}

#[test]
fn resolve_thread_sets_resolved() {
    let (_dir, repo) = test_repo();
    let (thread_id, root) = create_thread(&repo, "needs work", None, None, None).unwrap();

    let resolution = resolve_thread(&repo, &thread_id, &root.oid, Some("done"), None).unwrap();
    assert!(resolution.resolved);
    assert_eq!(resolution.body, "done");
    assert_eq!(resolution.reply_to.as_deref(), Some(root.oid.as_str()));
}

#[test]
fn edit_in_thread_sets_replaces() {
    let (_dir, repo) = test_repo();
    let (thread_id, root) = create_thread(&repo, "original", None, None, None).unwrap();

    let edited = edit_in_thread(&repo, &thread_id, &root.oid, "updated", None, None, None).unwrap();
    assert_eq!(edited.body, "updated");
    assert_eq!(edited.replaces.as_deref(), Some(root.oid.as_str()));
}

#[test]
fn list_thread_returns_tip_first() {
    let (_dir, repo) = test_repo();
    let (thread_id, root) = create_thread(&repo, "first", None, None, None).unwrap();
    let r1 = reply_to_thread(&repo, &thread_id, "second", &root.oid, None, None, None).unwrap();
    reply_to_thread(&repo, &thread_id, "third", &r1.oid, None, None, None).unwrap();

    let all = list_thread_comments(&repo, &thread_id).unwrap();
    assert_eq!(all.len(), 3);
    assert_eq!(all[0].body, "third");
    assert_eq!(all[2].body, "first");
}

#[test]
fn two_threads_no_contention() {
    let (_dir, repo) = test_repo();
    let (id1, _) = create_thread(&repo, "thread one", None, None, None).unwrap();
    let (id2, _) = create_thread(&repo, "thread two", None, None, None).unwrap();

    assert_ne!(id1, id2);

    let c1 = list_thread_comments(&repo, &id1).unwrap();
    let c2 = list_thread_comments(&repo, &id2).unwrap();
    assert_eq!(c1.len(), 1);
    assert_eq!(c2.len(), 1);
    assert_eq!(c1[0].body, "thread one");
    assert_eq!(c2[0].body, "thread two");
}

#[test]
fn comment_id_trailer_consistent() {
    let (_dir, repo) = test_repo();
    let (thread_id, root) = create_thread(&repo, "msg", None, None, None).unwrap();
    let ref_name = comment_thread_ref(&thread_id);
    let tip = repo
        .find_reference(&ref_name)
        .unwrap()
        .peel_to_commit()
        .unwrap();
    assert_eq!(root.oid, tip.id().to_string());
}

#[test]
fn anchor_trailer_on_every_commit() {
    let (_dir, repo) = test_repo();
    let blob_oid = repo.blob(b"anchor text").unwrap().to_string();
    let anchor = Anchor {
        oid: blob_oid,
        start_line: None,
        end_line: None,
    };
    let (thread_id, root) = create_thread(&repo, "first", Some(&anchor), None, None).unwrap();
    reply_to_thread(&repo, &thread_id, "second", &root.oid, None, None, None).unwrap();

    let ref_name = comment_thread_ref(&thread_id);
    let mut commit = repo
        .find_reference(&ref_name)
        .unwrap()
        .peel_to_commit()
        .unwrap();
    let mut count = 0;
    loop {
        let msg = commit.message().unwrap_or("");
        assert!(
            msg.contains("Anchor: "),
            "commit missing Anchor trailer: {msg}"
        );
        count += 1;
        if commit.parent_count() == 0 {
            break;
        }
        commit = commit.parent(0).unwrap();
    }
    assert_eq!(count, 2);
}

#[test]
fn anchor_v2_no_path_field() {
    let anchor = Anchor {
        oid: "abc1230000000000000000000000000000000000".to_string(),
        start_line: Some(1),
        end_line: Some(5),
    };
    assert_eq!(anchor.oid, "abc1230000000000000000000000000000000000");
    assert_eq!(anchor.start_line, Some(1));
    assert_eq!(anchor.end_line, Some(5));
}

#[test]
fn context_lines_roundtrip() {
    let (_dir, repo) = test_repo();
    let context = "fn main() {\n    println!(\"hello\");\n}";
    let blob_oid = repo.blob(context.as_bytes()).unwrap().to_string();
    let anchor = Anchor {
        oid: blob_oid,
        start_line: Some(1),
        end_line: Some(3),
    };
    let (thread_id, _) =
        create_thread(&repo, "context comment", Some(&anchor), Some(context), None).unwrap();

    let comments = list_thread_comments(&repo, &thread_id).unwrap();
    assert_eq!(comments[0].context_lines.as_deref(), Some(context));
}

// --- Index ---

#[test]
fn rebuild_index_and_lookup() {
    let (_dir, repo) = test_repo();
    let blob_oid = repo.blob(b"indexed file").unwrap().to_string();
    let anchor = Anchor {
        oid: blob_oid.clone(),
        start_line: None,
        end_line: None,
    };
    let (thread_id, _) = create_thread(&repo, "indexed", Some(&anchor), None, None).unwrap();

    rebuild_comments_index(&repo).unwrap();

    let threads = index_lookup(&repo, &blob_oid).unwrap();
    assert!(threads.is_some());
    assert!(threads.unwrap().contains(&thread_id));
}

#[test]
fn find_threads_by_object_uses_index() {
    let (_dir, repo) = test_repo();
    let blob_oid = repo.blob(b"indexed file 2").unwrap().to_string();
    let anchor = Anchor {
        oid: blob_oid.clone(),
        start_line: Some(5),
        end_line: Some(10),
    };
    let (thread_id, _) =
        create_thread(&repo, "indexed comment", Some(&anchor), None, None).unwrap();

    rebuild_comments_index(&repo).unwrap();

    let threads = find_threads_by_object(&repo, &blob_oid).unwrap();
    assert!(threads.contains(&thread_id));
}

#[test]
fn find_threads_fallback_without_index() {
    let (_dir, repo) = test_repo();
    let blob_oid = repo.blob(b"unindexed file").unwrap().to_string();
    let anchor = Anchor {
        oid: blob_oid.clone(),
        start_line: None,
        end_line: None,
    };
    let (thread_id, _) = create_thread(&repo, "unindexed", Some(&anchor), None, None).unwrap();

    // Do NOT rebuild the index — fallback scan must still find the thread.
    let threads = find_threads_by_object(&repo, &blob_oid).unwrap();
    assert!(threads.contains(&thread_id));
}

#[test]
fn find_threads_delete_index_falls_back_to_scan() {
    let (_dir, repo) = test_repo();
    let blob_oid = repo.blob(b"scannable file").unwrap().to_string();
    let anchor = Anchor {
        oid: blob_oid.clone(),
        start_line: None,
        end_line: None,
    };
    let (thread_id, _) = create_thread(&repo, "scan fallback", Some(&anchor), None, None).unwrap();

    rebuild_comments_index(&repo).unwrap();
    // Delete the index ref to force fallback.
    repo.find_reference(git_forge::refs::COMMENTS_INDEX)
        .unwrap()
        .delete()
        .unwrap();

    let threads = find_threads_by_object(&repo, &blob_oid).unwrap();
    assert!(threads.contains(&thread_id));
}

// --- Executor integration ---

#[test]
fn executor_create_and_list_comments_on_issue() {
    let (dir, repo) = test_repo();
    let store = Store::new(&repo);
    let issue = store
        .create_issue("Test issue", "body", &[], &[], None)
        .unwrap();
    let exec = Executor::from_path(dir.path()).unwrap();

    let anchor = git_forge::comment::Anchor {
        oid: issue.oid.clone(),
        start_line: None,
        end_line: None,
    };
    let (_, c1) = exec
        .create_comment("first", Some(&anchor), None, None)
        .unwrap();
    let (_, c2) = exec
        .create_comment("second", Some(&anchor), None, None)
        .unwrap();

    let comments = exec.list_comments_on(&issue.oid).unwrap();
    assert_eq!(comments.len(), 2);
    let oids: Vec<&str> = comments.iter().map(|c| c.oid.as_str()).collect();
    assert!(oids.contains(&c1.oid.as_str()));
    assert!(oids.contains(&c2.oid.as_str()));
}

#[test]
fn executor_reply_and_resolve_thread() {
    let (dir, repo) = test_repo();
    let store = Store::new(&repo);
    let issue = store
        .create_issue("Thread issue", "body", &[], &[], None)
        .unwrap();
    let exec = Executor::from_path(dir.path()).unwrap();

    let anchor = git_forge::comment::Anchor {
        oid: issue.oid.clone(),
        start_line: None,
        end_line: None,
    };
    let (thread_id, root) = exec
        .create_comment("root", Some(&anchor), None, None)
        .unwrap();
    let reply = exec
        .reply_comment(&thread_id, "reply text", &root.oid, None, None, None)
        .unwrap();
    assert_eq!(reply.reply_to.as_deref(), Some(root.oid.as_str()));

    let resolved = exec
        .resolve_comment_thread(&thread_id, &reply.oid, Some("done"), None)
        .unwrap();
    assert!(resolved.resolved);
    assert_eq!(resolved.body, "done");
}

#[test]
fn executor_create_comment_anchored_to_blob() {
    let (dir, repo) = test_repo();
    let blob_oid = repo.blob(b"file content\n").unwrap().to_string();
    let exec = Executor::from_path(dir.path()).unwrap();

    let anchor = git_forge::comment::Anchor {
        oid: blob_oid.clone(),
        start_line: Some(1),
        end_line: Some(1),
    };
    let (thread_id, _) = exec
        .create_comment("blob comment", Some(&anchor), None, None)
        .unwrap();

    let comments = exec.list_comments_on(&blob_oid).unwrap();
    assert_eq!(comments.len(), 1);
    let thread_ids: Vec<Option<&str>> = comments.iter().map(|c| c.thread_id.as_deref()).collect();
    assert!(thread_ids.contains(&Some(thread_id.as_str())));
}

#[test]
fn executor_retarget_does_not_migrate_comments() {
    let (dir, repo) = test_repo();
    let exec = Executor::from_path(dir.path()).unwrap();

    let old_content = b"line1\nline2\nline3\n";
    let new_content = b"line1\nLINE2\nline3\n";
    let old_blob_oid = repo.blob(old_content).unwrap().to_string();

    let mut tb = repo.treebuilder(None).unwrap();
    let oid = git2::Oid::from_str(&old_blob_oid).unwrap();
    tb.insert("a.rs", oid, 0o100_644).unwrap();
    let old_tree = tb.write().unwrap().to_string();

    let new_blob_oid = repo.blob(new_content).unwrap().to_string();
    let mut tb2 = repo.treebuilder(None).unwrap();
    let oid2 = git2::Oid::from_str(&new_blob_oid).unwrap();
    tb2.insert("a.rs", oid2, 0o100_644).unwrap();
    let new_tree = tb2.write().unwrap().to_string();

    let target = git_forge::review::ReviewTarget {
        head: old_tree,
        base: None,
        path: None,
    };
    let review = exec.create_review("r", "", &target, None, None).unwrap();

    let anchor = git_forge::comment::Anchor {
        oid: old_blob_oid.clone(),
        start_line: Some(2),
        end_line: Some(2),
    };
    exec.create_comment("on line 2", Some(&anchor), None, None)
        .unwrap();

    exec.retarget_review(&review.oid, Some(&new_tree), None)
        .unwrap();

    // Comments are still anchored to old_blob_oid — NOT migrated.
    let old_comments = exec.list_comments_on(&old_blob_oid).unwrap();
    assert_eq!(old_comments.len(), 1);
    let new_comments = exec.list_comments_on(&new_blob_oid).unwrap();
    assert!(new_comments.is_empty());
}

#[test]
fn executor_resolve_anchor_spec_raw_oid() {
    let (dir, repo) = test_repo();
    let blob_oid = repo.blob(b"x").unwrap().to_string();
    let exec = Executor::from_path(dir.path()).unwrap();
    assert_eq!(exec.resolve_anchor_spec(&blob_oid).unwrap(), blob_oid);
}

#[test]
fn executor_resolve_anchor_spec_issue() {
    let (dir, repo) = test_repo();
    let store = Store::new(&repo);
    let issue = store
        .create_issue("spec issue", "body", &[], &[], None)
        .unwrap();
    let exec = Executor::from_path(dir.path()).unwrap();
    let spec = format!(
        "issue:{}",
        issue.display_id.as_deref().unwrap_or(&issue.oid)
    );
    assert_eq!(exec.resolve_anchor_spec(&spec).unwrap(), issue.oid);
}
