//! Regression tests for executor fixes.
#![allow(clippy::must_use_candidate, clippy::missing_panics_doc, missing_docs)]

use std::fs;
use std::path::Path;

use git2::Repository;
use tempfile::TempDir;

use git_forge::Error;
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
