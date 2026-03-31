//! Regression tests for executor fixes.
#![allow(
    clippy::must_use_candidate,
    clippy::missing_panics_doc,
    missing_docs,
    deprecated
)]

use std::fs;
use std::path::Path;

use git2::Repository;
use tempfile::TempDir;

use git_forge::Error;
use git_forge::comment::LegacyAnchor as Anchor;
use git_forge::exe::Executor;
use git_forge::review::ReviewTarget;

fn test_repo() -> (TempDir, Repository) {
    let dir = TempDir::new().expect("temp dir");
    let repo = Repository::init(dir.path()).expect("init repo");
    {
        let mut cfg = repo.config().expect("config");
        cfg.set_str("user.name", "test").expect("user.name");
        cfg.set_str("user.email", "test@test.com")
            .expect("user.email");
    }
    fs::write(dir.path().join("hello.txt"), "hello\n").unwrap();
    {
        let sig = git2::Signature::now("test", "test@test.com").expect("sig");
        let mut index = repo.index().expect("index");
        index.add_path(Path::new("hello.txt")).expect("add");
        let tree_oid = index.write_tree().expect("write tree");
        let tree = repo.find_tree(tree_oid).expect("find tree");
        repo.commit(Some("HEAD"), &sig, &sig, "initial", &tree, &[])
            .expect("initial commit");
    }
    (dir, repo)
}

fn head_oid(repo: &Repository) -> String {
    repo.head()
        .unwrap()
        .peel_to_commit()
        .unwrap()
        .id()
        .to_string()
}

fn head_tree_oid(repo: &Repository) -> String {
    repo.head()
        .unwrap()
        .peel_to_commit()
        .unwrap()
        .tree_id()
        .to_string()
}

// ---------------------------------------------------------------------------
// hash_worktree_dir: .gitignore
// ---------------------------------------------------------------------------

#[test]
fn hash_worktree_dir_skips_gitignored_files() {
    let (dir, repo) = test_repo();
    fs::write(dir.path().join(".gitignore"), "*.log\n").unwrap();
    fs::write(dir.path().join("tracked.txt"), "tracked\n").unwrap();
    fs::write(dir.path().join("ignored.log"), "should be ignored\n").unwrap();

    let oid = git_forge::exe::hash_worktree_dir(&repo, dir.path()).unwrap();
    let tree = repo.find_tree(oid).unwrap();
    assert!(tree.get_name("tracked.txt").is_some());
    assert!(
        tree.get_name("ignored.log").is_none(),
        "gitignored file should not appear in hashed tree"
    );
}

// ---------------------------------------------------------------------------
// hash_worktree_dir: .git skipped
// ---------------------------------------------------------------------------

#[test]
fn hash_worktree_dir_skips_dot_git() {
    let (dir, repo) = test_repo();

    let oid = git_forge::exe::hash_worktree_dir(&repo, dir.path()).unwrap();
    let tree = repo.find_tree(oid).unwrap();
    assert!(
        tree.get_name(".git").is_none(),
        ".git should never appear in hashed tree"
    );
}

// ---------------------------------------------------------------------------
// hash_worktree_dir: symlinks
// ---------------------------------------------------------------------------

#[cfg(unix)]
#[test]
fn hash_worktree_dir_follows_symlinks() {
    let (dir, repo) = test_repo();
    fs::write(dir.path().join("real.txt"), "real content\n").unwrap();
    std::os::unix::fs::symlink(dir.path().join("real.txt"), dir.path().join("link.txt")).unwrap();

    let oid = git_forge::exe::hash_worktree_dir(&repo, dir.path()).unwrap();
    let tree = repo.find_tree(oid).unwrap();

    let real_entry = tree.get_name("real.txt").expect("real.txt present");
    let link_entry = tree
        .get_name("link.txt")
        .expect("symlink should be followed");
    assert_eq!(real_entry.id(), link_entry.id());
}

// ---------------------------------------------------------------------------
// hash_worktree_dir: executable bit
// ---------------------------------------------------------------------------

#[cfg(unix)]
#[test]
fn hash_worktree_dir_preserves_executable_bit() {
    use std::os::unix::fs::PermissionsExt;

    let (dir, repo) = test_repo();
    let script = dir.path().join("run.sh");
    fs::write(&script, "#!/bin/sh\necho hi\n").unwrap();
    fs::set_permissions(&script, fs::Permissions::from_mode(0o755)).unwrap();

    let data = dir.path().join("data.txt");
    fs::write(&data, "data\n").unwrap();

    let oid = git_forge::exe::hash_worktree_dir(&repo, dir.path()).unwrap();
    let tree = repo.find_tree(oid).unwrap();

    assert_eq!(tree.get_name("run.sh").unwrap().filemode(), 0o100_755);
    assert_eq!(tree.get_name("data.txt").unwrap().filemode(), 0o100_644);
}

// ---------------------------------------------------------------------------
// resolve_path: clean path returns HEAD OID
// ---------------------------------------------------------------------------

#[test]
fn resolve_path_clean_returns_head_oid() {
    let (dir, repo) = test_repo();
    let exec = Executor::from_path(dir.path()).unwrap();

    let oid = exec.resolve_path(Path::new("hello.txt"), false).unwrap();
    // Should match the blob OID in HEAD.
    let expected = repo
        .revparse_single("HEAD:hello.txt")
        .unwrap()
        .id()
        .to_string();
    assert_eq!(oid, expected);
}

// ---------------------------------------------------------------------------
// resolve_path: dirty without allow_dirty → error
// ---------------------------------------------------------------------------

#[test]
fn resolve_path_dirty_without_allow_returns_error() {
    let (dir, _repo) = test_repo();
    let exec = Executor::from_path(dir.path()).unwrap();

    fs::write(dir.path().join("hello.txt"), "modified\n").unwrap();
    let result = exec.resolve_path(Path::new("hello.txt"), false);
    assert!(matches!(result, Err(Error::DirtyWorktree)));
}

// ---------------------------------------------------------------------------
// resolve_path: dirty with allow_dirty → hashed OID
// ---------------------------------------------------------------------------

#[test]
fn resolve_path_dirty_with_allow_returns_working_oid() {
    let (dir, repo) = test_repo();
    let exec = Executor::from_path(dir.path()).unwrap();

    fs::write(dir.path().join("hello.txt"), "modified\n").unwrap();
    let oid = exec.resolve_path(Path::new("hello.txt"), true).unwrap();

    // Should match the blob of the working-tree content.
    let expected = repo
        .blob_path(&dir.path().join("hello.txt"))
        .unwrap()
        .to_string();
    assert_eq!(oid, expected);
}

// ---------------------------------------------------------------------------
// resolve_head: plain HEAD with allow_dirty hashes worktree
// ---------------------------------------------------------------------------

#[test]
fn resolve_head_dirty_hashes_worktree() {
    let (dir, repo) = test_repo();
    let exec = Executor::from_path(dir.path()).unwrap();

    fs::write(dir.path().join("hello.txt"), "modified\n").unwrap();
    let oid = exec.resolve_head("HEAD", true).unwrap();

    // Should NOT match HEAD's commit or tree since worktree is dirty.
    assert_ne!(oid, head_oid(&repo));
    assert_ne!(oid, head_tree_oid(&repo));
    // Should be a valid tree object.
    let obj = repo
        .find_object(git2::Oid::from_str(&oid).unwrap(), None)
        .unwrap();
    assert_eq!(obj.kind(), Some(git2::ObjectType::Tree));
}

// ---------------------------------------------------------------------------
// resolve_head: HEAD^{tree} with allow_dirty also hashes worktree
// ---------------------------------------------------------------------------

#[test]
fn resolve_head_tree_peel_dirty_hashes_worktree() {
    let (dir, _repo) = test_repo();
    let exec = Executor::from_path(dir.path()).unwrap();

    fs::write(dir.path().join("hello.txt"), "modified\n").unwrap();

    let from_head = exec.resolve_head("HEAD", true).unwrap();
    let from_tree = exec.resolve_head("HEAD^{tree}", true).unwrap();

    // Both should produce the same dirty worktree hash.
    assert_eq!(from_head, from_tree);
}

// ---------------------------------------------------------------------------
// resolve_head: non-HEAD ref is unaffected by allow_dirty
// ---------------------------------------------------------------------------

#[test]
fn resolve_head_non_head_ref_ignores_dirty() {
    let (dir, repo) = test_repo();

    // Create a second commit on a branch.
    let sig = git2::Signature::now("test", "test@test.com").unwrap();
    let head = repo.head().unwrap().peel_to_commit().unwrap();
    let blob_oid = repo.blob(b"branch content").unwrap();
    let mut builder = repo.treebuilder(Some(&head.tree().unwrap())).unwrap();
    builder.insert("branch.txt", blob_oid, 0o100_644).unwrap();
    let tree_oid = builder.write().unwrap();
    let tree = repo.find_tree(tree_oid).unwrap();
    let branch_commit = repo
        .commit(None, &sig, &sig, "branch commit", &tree, &[&head])
        .unwrap();
    repo.reference("refs/heads/other", branch_commit, false, "create branch")
        .unwrap();

    // Dirty the worktree.
    fs::write(dir.path().join("hello.txt"), "modified\n").unwrap();

    let exec = Executor::from_path(dir.path()).unwrap();
    let oid = exec.resolve_head("other", true).unwrap();
    // Should return the branch commit OID, not a dirty hash.
    assert_eq!(oid, branch_commit.to_string());
}

// ---------------------------------------------------------------------------
// resolve_head: without allow_dirty returns plain OID
// ---------------------------------------------------------------------------

#[test]
fn resolve_head_without_allow_dirty_returns_plain() {
    let (dir, repo) = test_repo();
    let exec = Executor::from_path(dir.path()).unwrap();

    let oid = exec.resolve_head("HEAD", false).unwrap();
    assert_eq!(oid, head_oid(&repo));
}

// ---------------------------------------------------------------------------
// resolve_comment_entity: object rejects tree
// ---------------------------------------------------------------------------

#[test]
fn resolve_comment_entity_rejects_tree() {
    let (dir, repo) = test_repo();
    let exec = Executor::from_path(dir.path()).unwrap();

    let tree_oid = head_tree_oid(&repo);
    let result = exec.resolve_comment_entity(None, None, Some(&tree_oid));
    assert!(
        matches!(&result, Err(Error::InvalidObjectType(_))),
        "expected InvalidObjectType, got {result:?}"
    );
}

// ---------------------------------------------------------------------------
// resolve_comment_entity: object accepts commit
// ---------------------------------------------------------------------------

#[test]
fn resolve_comment_entity_accepts_commit() {
    let (dir, repo) = test_repo();
    let exec = Executor::from_path(dir.path()).unwrap();

    let oid = head_oid(&repo);
    let ref_name = exec.resolve_comment_entity(None, None, Some(&oid)).unwrap();
    assert!(ref_name.contains("refs/forge/comments/object/"));
}

// ---------------------------------------------------------------------------
// resolve_comment_entity: object accepts blob
// ---------------------------------------------------------------------------

#[test]
fn resolve_comment_entity_accepts_blob() {
    let (dir, repo) = test_repo();
    let exec = Executor::from_path(dir.path()).unwrap();

    let blob_oid = repo.blob(b"test blob").unwrap().to_string();
    let ref_name = exec
        .resolve_comment_entity(None, None, Some(&blob_oid))
        .unwrap();
    assert!(ref_name.contains("refs/forge/comments/object/"));
}

// ---------------------------------------------------------------------------
// resolve_comment_entity: no args → error
// ---------------------------------------------------------------------------

#[test]
fn resolve_comment_entity_no_args_errors() {
    let (dir, _repo) = test_repo();
    let exec = Executor::from_path(dir.path()).unwrap();

    let result = exec.resolve_comment_entity(None, None, None);
    assert!(matches!(result, Err(Error::Config(_))));
}

// ---------------------------------------------------------------------------
// object comments: round-trip through Executor
// ---------------------------------------------------------------------------

#[test]
fn object_comment_roundtrip() {
    let (dir, repo) = test_repo();
    let exec = Executor::from_path(dir.path()).unwrap();
    let oid = head_oid(&repo);

    exec.add_object_comment(&oid, "hello", None).unwrap();
    let comments = exec.list_object_comments(&oid).unwrap();
    assert_eq!(comments.len(), 1);
    assert_eq!(comments[0].body, "hello");
}

// ---------------------------------------------------------------------------
// object comment: reject bare tree via Executor
// ---------------------------------------------------------------------------

#[test]
fn add_object_comment_rejects_tree() {
    let (dir, repo) = test_repo();
    let exec = Executor::from_path(dir.path()).unwrap();

    let tree_oid = head_tree_oid(&repo);
    let result = exec.add_object_comment(&tree_oid, "nope", None);
    assert!(matches!(&result, Err(Error::InvalidObjectType(_))));
}

// ---------------------------------------------------------------------------
// should_interact: FORGE_NO_INTERACTIVE suppresses
// ---------------------------------------------------------------------------

#[test]
fn should_interact_returns_false_when_not_missing_input() {
    assert!(!git_forge::exe::should_interact(false));
}

#[test]
fn should_interact_returns_false_without_tty() {
    // Tests run without a TTY, so should_interact(true) is still false.
    assert!(!git_forge::exe::should_interact(true));
}

// ---------------------------------------------------------------------------
// checkout_review: creates worktree in a new parent directory
// (regression: create_dir_all created the leaf, but libgit2 also creates it)
// ---------------------------------------------------------------------------

#[test]
fn checkout_review_creates_parent_dir_only() {
    let (dir, repo) = test_repo();
    let exec = Executor::from_path(dir.path()).unwrap();

    let target = ReviewTarget {
        head: head_oid(&repo),
        base: None,
    };
    let review = exec
        .create_review("checkout test", "", &target, None)
        .unwrap();

    let wt_dir = dir.path().join("review-wt");
    let (_, wt_path) = exec.checkout_review(&review.oid, Some(&wt_dir)).unwrap();
    assert_eq!(wt_path, wt_dir);
    assert!(wt_dir.join(".git").exists());
}

// ---------------------------------------------------------------------------
// checkout_review: recovers from stale worktree admin directory
// (regression: orphaned .git/worktrees/<name> blocked re-checkout)
// ---------------------------------------------------------------------------

#[test]
fn checkout_review_recovers_from_stale_admin_dir() {
    let (dir, repo) = test_repo();
    let exec = Executor::from_path(dir.path()).unwrap();

    let target = ReviewTarget {
        head: head_oid(&repo),
        base: None,
    };
    let review = exec.create_review("stale test", "", &target, None).unwrap();

    let wt_name = format!("forge-review-{}", &review.oid[..12]);

    // Simulate an orphaned admin directory (leftover from a failed checkout).
    let stale = repo.path().join("worktrees").join(&wt_name);
    fs::create_dir_all(&stale).unwrap();
    fs::write(stale.join("gitdir"), "garbage").unwrap();

    let wt_dir = dir.path().join("review-wt");
    let (_, wt_path) = exec.checkout_review(&review.oid, Some(&wt_dir)).unwrap();
    assert_eq!(wt_path, wt_dir);
    assert!(wt_dir.join(".git").exists());
}

// ---------------------------------------------------------------------------
// checkout_review: idempotent when worktree already valid
// ---------------------------------------------------------------------------

#[test]
fn checkout_review_idempotent() {
    let (dir, repo) = test_repo();
    let exec = Executor::from_path(dir.path()).unwrap();

    let target = ReviewTarget {
        head: head_oid(&repo),
        base: None,
    };
    let review = exec
        .create_review("idempotent test", "", &target, None)
        .unwrap();

    let wt_dir = dir.path().join("review-wt");
    let (_, first) = exec.checkout_review(&review.oid, Some(&wt_dir)).unwrap();
    let (_, second) = exec.checkout_review(&review.oid, Some(&wt_dir)).unwrap();
    assert_eq!(
        first.canonicalize().unwrap(),
        second.canonicalize().unwrap()
    );
}

// ---------------------------------------------------------------------------
// checkout_review + done_review: round-trip then re-checkout
// ---------------------------------------------------------------------------

#[test]
fn checkout_done_recheckout() {
    let (dir, repo) = test_repo();
    let exec = Executor::from_path(dir.path()).unwrap();

    let target = ReviewTarget {
        head: head_oid(&repo),
        base: None,
    };
    let review = exec
        .create_review("roundtrip test", "", &target, None)
        .unwrap();

    let wt_dir = dir.path().join("review-wt");
    exec.checkout_review(&review.oid, Some(&wt_dir)).unwrap();
    exec.done_review(Some(&review.oid)).unwrap();
    assert!(!wt_dir.exists());

    let (_, wt_path) = exec.checkout_review(&review.oid, Some(&wt_dir)).unwrap();
    assert_eq!(wt_path, wt_dir);
    assert!(wt_dir.join(".git").exists());
}

// ---------------------------------------------------------------------------
// read-only commands don't require clean worktree
// (regression: check_clean was at top of dispatch)
// ---------------------------------------------------------------------------

#[test]
fn list_issues_works_on_dirty_worktree() {
    let (dir, _repo) = test_repo();
    // Dirty the worktree.
    fs::write(dir.path().join("hello.txt"), "dirty\n").unwrap();

    let exec = Executor::from_path(dir.path()).unwrap();
    // list_issues should work fine on a dirty worktree.
    let issues = exec.list_issues(None).unwrap();
    assert!(issues.is_empty());
}

// ---------------------------------------------------------------------------
// retarget: basic head update
// ---------------------------------------------------------------------------

/// Build a tree with a single blob at `name` containing `content`.
fn make_tree(repo: &Repository, name: &str, content: &str) -> String {
    let blob_oid = repo.blob(content.as_bytes()).unwrap();
    let mut builder = repo.treebuilder(None).unwrap();
    builder.insert(name, blob_oid, 0o100_644).unwrap();
    builder.write().unwrap().to_string()
}

/// Build a tree with multiple `(name, content)` entries.
fn make_tree_multi(repo: &Repository, files: &[(&str, &str)]) -> String {
    let mut builder = repo.treebuilder(None).unwrap();
    for (name, content) in files {
        let blob_oid = repo.blob(content.as_bytes()).unwrap();
        builder.insert(*name, blob_oid, 0o100_644).unwrap();
    }
    builder.write().unwrap().to_string()
}

#[test]
fn retarget_updates_head() {
    let (dir, repo) = test_repo();
    let exec = Executor::from_path(dir.path()).unwrap();

    let old_tree = make_tree(&repo, "lib.rs", "fn main() {}\n");
    let target = ReviewTarget {
        head: old_tree.clone(),
        base: None,
    };
    let review = exec
        .create_review("test review", "", &target, None)
        .unwrap();

    let new_tree = make_tree(&repo, "lib.rs", "fn main() { todo!() }\n");
    let (updated, _) = exec.retarget_review(&review.oid, &new_tree).unwrap();
    assert_eq!(updated.target.head, new_tree);
    assert_ne!(updated.target.head, old_tree);
}

// ---------------------------------------------------------------------------
// retarget: comment migration with content matching
// ---------------------------------------------------------------------------

#[test]
fn retarget_migrates_carry_forward_comments() {
    let (dir, repo) = test_repo();
    let exec = Executor::from_path(dir.path()).unwrap();

    // Old tree: two files, comment on line 2-3 of a.rs.
    let old_tree = make_tree_multi(
        &repo,
        &[
            ("a.rs", "line1\nline2\nline3\nline4\n"),
            ("b.rs", "unchanged content\n"),
        ],
    );
    let target = ReviewTarget {
        head: old_tree.clone(),
        base: None,
    };
    let review = exec
        .create_review("migration test", "", &target, None)
        .unwrap();

    // Find the blob OID for a.rs in the old tree.
    let old_tree_obj = repo
        .find_tree(git2::Oid::from_str(&old_tree).unwrap())
        .unwrap();
    let old_a_blob = old_tree_obj.get_name("a.rs").unwrap().id().to_string();

    // Add a comment anchored to blob a.rs at lines 2-3.
    let anchor = Anchor::Object {
        oid: old_a_blob.clone(),
        path: Some("a.rs".to_string()),
        range: Some("2-3".to_string()),
    };
    let original = exec
        .add_review_comment(&review.oid, "needs refactor", Some(&anchor))
        .unwrap();

    // New tree: a.rs has a new first line but lines 2-3 are preserved.
    // b.rs is unchanged.
    let new_tree = make_tree_multi(
        &repo,
        &[
            ("a.rs", "new_line1\nline2\nline3\nline4\n"),
            ("b.rs", "unchanged content\n"),
        ],
    );

    let (_, migrated) = exec.retarget_review(&review.oid, &new_tree).unwrap();
    assert_eq!(migrated, 1, "one comment should be migrated");

    // The migrated comment should appear when listing review comments.
    let comments = exec.list_review_comments(&review.oid).unwrap();
    assert!(!comments.is_empty(), "should have comments after retarget");

    // Find the migrated comment (the one with migrated_from set).
    let migrated_comment = comments.iter().find(|c| c.migrated_from.is_some()).unwrap();
    assert_eq!(migrated_comment.body, "needs refactor");
    // migrated_from records the original comment chain OID, not the blob OID.
    assert_eq!(
        migrated_comment.migrated_from.as_deref(),
        Some(original.oid.as_str())
    );

    // Check the new anchor has updated range (shifted by 1 line).
    if let Some(Anchor::Object { range, .. }) = &migrated_comment.anchor {
        assert_eq!(range.as_deref(), Some("2-3"));
    } else {
        panic!("expected object anchor on migrated comment");
    }
}

// ---------------------------------------------------------------------------
// retarget: comments on changed context are NOT migrated
// ---------------------------------------------------------------------------

#[test]
fn retarget_drops_comments_on_changed_context() {
    let (dir, repo) = test_repo();
    let exec = Executor::from_path(dir.path()).unwrap();

    let old_tree = make_tree(&repo, "a.rs", "line1\nline2\nline3\n");
    let target = ReviewTarget {
        head: old_tree.clone(),
        base: None,
    };
    let review = exec.create_review("drop test", "", &target, None).unwrap();

    let old_tree_obj = repo
        .find_tree(git2::Oid::from_str(&old_tree).unwrap())
        .unwrap();
    let old_blob = old_tree_obj.get_name("a.rs").unwrap().id().to_string();

    let anchor = Anchor::Object {
        oid: old_blob.clone(),
        path: Some("a.rs".to_string()),
        range: Some("2-3".to_string()),
    };
    exec.add_review_comment(&review.oid, "old comment", Some(&anchor))
        .unwrap();

    // New tree: lines 2-3 have completely changed.
    let new_tree = make_tree(&repo, "a.rs", "line1\nNEW2\nNEW3\n");
    let (_, migrated) = exec.retarget_review(&review.oid, &new_tree).unwrap();
    assert_eq!(
        migrated, 0,
        "comment should not migrate when context changed"
    );
}

// ---------------------------------------------------------------------------
// retarget: file-level (no range) comments migrate unconditionally
// ---------------------------------------------------------------------------

#[test]
fn retarget_migrates_file_level_comments() {
    let (dir, repo) = test_repo();
    let exec = Executor::from_path(dir.path()).unwrap();

    let old_tree = make_tree(&repo, "a.rs", "old content\n");
    let target = ReviewTarget {
        head: old_tree.clone(),
        base: None,
    };
    let review = exec
        .create_review("file level test", "", &target, None)
        .unwrap();

    let old_tree_obj = repo
        .find_tree(git2::Oid::from_str(&old_tree).unwrap())
        .unwrap();
    let old_blob = old_tree_obj.get_name("a.rs").unwrap().id().to_string();

    let anchor = Anchor::Object {
        oid: old_blob.clone(),
        path: Some("a.rs".to_string()),
        range: None,
    };
    exec.add_review_comment(&review.oid, "file-level note", Some(&anchor))
        .unwrap();

    let new_tree = make_tree(&repo, "a.rs", "completely new content\n");
    let (_, migrated) = exec.retarget_review(&review.oid, &new_tree).unwrap();
    assert_eq!(migrated, 1, "file-level comment should always migrate");
}

// ---------------------------------------------------------------------------
// retarget: unchanged blobs keep comments without migration
// ---------------------------------------------------------------------------

#[test]
fn retarget_unchanged_blobs_keep_comments() {
    let (dir, repo) = test_repo();
    let exec = Executor::from_path(dir.path()).unwrap();

    let old_tree = make_tree_multi(&repo, &[("a.rs", "unchanged\n"), ("b.rs", "will change\n")]);
    let target = ReviewTarget {
        head: old_tree.clone(),
        base: None,
    };
    let review = exec
        .create_review("unchanged test", "", &target, None)
        .unwrap();

    let old_tree_obj = repo
        .find_tree(git2::Oid::from_str(&old_tree).unwrap())
        .unwrap();
    let a_blob = old_tree_obj.get_name("a.rs").unwrap().id().to_string();

    let anchor = Anchor::Object {
        oid: a_blob.clone(),
        path: Some("a.rs".to_string()),
        range: Some("1-1".to_string()),
    };
    exec.add_review_comment(&review.oid, "stays put", Some(&anchor))
        .unwrap();

    // Only change b.rs.
    let new_tree = make_tree_multi(&repo, &[("a.rs", "unchanged\n"), ("b.rs", "changed!\n")]);
    let (_, migrated) = exec.retarget_review(&review.oid, &new_tree).unwrap();
    assert_eq!(migrated, 0, "unchanged blob needs no migration");

    let comments = exec.list_review_comments(&review.oid).unwrap();
    assert_eq!(comments.len(), 1);
    assert_eq!(comments[0].body, "stays put");
    assert!(comments[0].migrated_from.is_none());
}

// ---------------------------------------------------------------------------
// retarget: insertion before anchor shifts range down
// ---------------------------------------------------------------------------

#[test]
fn retarget_insertion_before_anchor_shifts_range_down() {
    let (dir, repo) = test_repo();
    let exec = Executor::from_path(dir.path()).unwrap();

    // 5-line file; anchor at lines 3-4.
    let old_tree = make_tree(&repo, "a.rs", "a\nb\nc\nd\ne\n");
    let target = ReviewTarget {
        head: old_tree.clone(),
        base: None,
    };
    let review = exec.create_review("shift down", "", &target, None).unwrap();

    let old_blob = repo
        .find_tree(git2::Oid::from_str(&old_tree).unwrap())
        .unwrap()
        .get_name("a.rs")
        .unwrap()
        .id()
        .to_string();

    exec.add_review_comment(
        &review.oid,
        "anchor",
        Some(&Anchor::Object {
            oid: old_blob.clone(),
            path: Some("a.rs".to_string()),
            range: Some("3-4".to_string()),
        }),
    )
    .unwrap();

    // Prepend 2 lines — anchor should shift to 5-6.
    let new_tree = make_tree(&repo, "a.rs", "x\ny\na\nb\nc\nd\ne\n");
    let (_, migrated) = exec.retarget_review(&review.oid, &new_tree).unwrap();
    assert_eq!(migrated, 1);

    let comments = exec.list_review_comments(&review.oid).unwrap();
    let m = comments.iter().find(|c| c.migrated_from.is_some()).unwrap();
    if let Some(Anchor::Object { range, .. }) = &m.anchor {
        assert_eq!(range.as_deref(), Some("5-6"));
    } else {
        panic!("expected object anchor");
    }
}

// ---------------------------------------------------------------------------
// retarget: deletion before anchor shifts range up
// ---------------------------------------------------------------------------

#[test]
fn retarget_deletion_before_anchor_shifts_range_up() {
    let (dir, repo) = test_repo();
    let exec = Executor::from_path(dir.path()).unwrap();

    let old_tree = make_tree(&repo, "a.rs", "a\nb\nc\nd\ne\n");
    let target = ReviewTarget {
        head: old_tree.clone(),
        base: None,
    };
    let review = exec.create_review("shift up", "", &target, None).unwrap();

    let old_blob = repo
        .find_tree(git2::Oid::from_str(&old_tree).unwrap())
        .unwrap()
        .get_name("a.rs")
        .unwrap()
        .id()
        .to_string();

    exec.add_review_comment(
        &review.oid,
        "anchor",
        Some(&Anchor::Object {
            oid: old_blob.clone(),
            path: Some("a.rs".to_string()),
            range: Some("4-5".to_string()),
        }),
    )
    .unwrap();

    // Remove line 2 ("b") — anchor should shift to 3-4.
    let new_tree = make_tree(&repo, "a.rs", "a\nc\nd\ne\n");
    let (_, migrated) = exec.retarget_review(&review.oid, &new_tree).unwrap();
    assert_eq!(migrated, 1);

    let comments = exec.list_review_comments(&review.oid).unwrap();
    let m = comments.iter().find(|c| c.migrated_from.is_some()).unwrap();
    if let Some(Anchor::Object { range, .. }) = &m.anchor {
        assert_eq!(range.as_deref(), Some("3-4"));
    } else {
        panic!("expected object anchor");
    }
}

// ---------------------------------------------------------------------------
// retarget: multiple hunks produce cumulative offset
// ---------------------------------------------------------------------------

#[test]
fn retarget_multiple_hunks_cumulative_offset() {
    let (dir, repo) = test_repo();
    let exec = Executor::from_path(dir.path()).unwrap();

    // 10-line file; anchor at lines 8-9.
    let old_tree = make_tree(&repo, "a.rs", "1\n2\n3\n4\n5\n6\n7\n8\n9\n10\n");
    let target = ReviewTarget {
        head: old_tree.clone(),
        base: None,
    };
    let review = exec.create_review("multi hunk", "", &target, None).unwrap();

    let old_blob = repo
        .find_tree(git2::Oid::from_str(&old_tree).unwrap())
        .unwrap()
        .get_name("a.rs")
        .unwrap()
        .id()
        .to_string();

    exec.add_review_comment(
        &review.oid,
        "anchor",
        Some(&Anchor::Object {
            oid: old_blob.clone(),
            path: Some("a.rs".to_string()),
            range: Some("8-9".to_string()),
        }),
    )
    .unwrap();

    // +2 at line 2 (insert x,y after "1"), -1 at line 6 (remove "5" equivalent).
    // New: "1\nx\ny\n2\n3\n4\n6\n7\n8\n9\n10\n" → anchor lines 8-9 ("7","8")
    // shift: +2 from insertion, -1 from deletion = net +1 → new range 9-10.
    let new_tree = make_tree(&repo, "a.rs", "1\nx\ny\n2\n3\n4\n6\n7\n8\n9\n10\n");
    let (_, migrated) = exec.retarget_review(&review.oid, &new_tree).unwrap();
    assert_eq!(migrated, 1);

    let comments = exec.list_review_comments(&review.oid).unwrap();
    let m = comments.iter().find(|c| c.migrated_from.is_some()).unwrap();
    if let Some(Anchor::Object { range, .. }) = &m.anchor {
        assert_eq!(range.as_deref(), Some("9-10"));
    } else {
        panic!("expected object anchor");
    }
}

// ---------------------------------------------------------------------------
// retarget: resolved comments are not migrated
// ---------------------------------------------------------------------------

#[test]
fn retarget_resolved_comments_not_migrated() {
    let (dir, repo) = test_repo();
    let exec = Executor::from_path(dir.path()).unwrap();

    let old_tree = make_tree(&repo, "a.rs", "a\nb\nc\n");
    let target = ReviewTarget {
        head: old_tree.clone(),
        base: None,
    };
    let review = exec.create_review("resolved", "", &target, None).unwrap();

    let old_blob = repo
        .find_tree(git2::Oid::from_str(&old_tree).unwrap())
        .unwrap()
        .get_name("a.rs")
        .unwrap()
        .id()
        .to_string();

    let anchor = Anchor::Object {
        oid: old_blob.clone(),
        path: Some("a.rs".to_string()),
        range: Some("2-3".to_string()),
    };
    let comment = exec
        .add_review_comment(&review.oid, "will be resolved", Some(&anchor))
        .unwrap();

    // Resolve the comment thread.
    exec.resolve_object_comment(&old_blob, &comment.oid, Some("fixed"))
        .unwrap();

    // Append a line (shifts anchor if not resolved).
    let new_tree = make_tree(&repo, "a.rs", "z\na\nb\nc\n");
    let (_, migrated) = exec.retarget_review(&review.oid, &new_tree).unwrap();
    assert_eq!(migrated, 0, "resolved comments must not be migrated");
}
