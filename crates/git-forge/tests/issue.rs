//! Tests for the issue subcommand.

use std::process::Command;

use tempfile::TempDir;

fn setup_repo() -> TempDir {
    let dir = TempDir::new().unwrap();
    Command::new("git")
        .args(["init"])
        .current_dir(dir.path())
        .output()
        .unwrap();
    Command::new("git")
        .args(["config", "user.email", "test@example.com"])
        .current_dir(dir.path())
        .output()
        .unwrap();
    Command::new("git")
        .args(["config", "user.name", "Test User"])
        .current_dir(dir.path())
        .output()
        .unwrap();
    Command::new("git")
        .args(["commit", "--allow-empty", "-m", "init"])
        .current_dir(dir.path())
        .output()
        .unwrap();
    dir
}

fn cmd(dir: &TempDir) -> assert_cmd::Command {
    let mut c = assert_cmd::Command::cargo_bin("forge").unwrap();
    c.current_dir(dir.path());
    c
}

// --- issue new ---

#[test]
fn new_issue_succeeds() {
    let dir = setup_repo();
    cmd(&dir)
        .args(["issue", "new", "My first issue", "--body", "some body"])
        .assert()
        .success()
        .stderr(predicates::str::contains("Created issue #1"));
}

#[test]
fn new_issue_sequential_ids() {
    let dir = setup_repo();
    cmd(&dir)
        .args(["issue", "new", "First", "--body", ""])
        .assert()
        .success()
        .stderr(predicates::str::contains("Created issue #1"));
    cmd(&dir)
        .args(["issue", "new", "Second", "--body", ""])
        .assert()
        .success()
        .stderr(predicates::str::contains("Created issue #2"));
}

#[test]
fn new_issue_with_labels() {
    let dir = setup_repo();
    cmd(&dir)
        .args(["issue", "new", "Labeled", "--body", "body"])
        .assert()
        .success()
        .stderr(predicates::str::contains("Created issue #1"));
    cmd(&dir)
        .args(["issue", "label", "1", "--add", "bug", "--add", "help wanted"])
        .assert()
        .success()
        .stderr(predicates::str::contains("Updated labels on issue #1"));
}

// --- issue list ---

#[test]
fn list_empty_repo_prints_no_issues() {
    let dir = setup_repo();
    cmd(&dir)
        .args(["issue", "list"])
        .assert()
        .success()
        .stdout(predicates::str::contains("No open issues."));
}

#[test]
fn list_shows_created_issues() {
    let dir = setup_repo();
    cmd(&dir)
        .args(["issue", "new", "Alpha", "--body", ""])
        .assert()
        .success();
    cmd(&dir)
        .args(["issue", "new", "Beta", "--body", ""])
        .assert()
        .success();
    let out = cmd(&dir).args(["issue", "list"]).assert().success();
    let stdout = String::from_utf8(out.get_output().stdout.clone()).unwrap();
    assert!(stdout.contains("Alpha"));
    assert!(stdout.contains("Beta"));
}

#[test]
fn list_closed_empty_when_none_closed() {
    let dir = setup_repo();
    cmd(&dir)
        .args(["issue", "new", "Open one", "--body", ""])
        .assert()
        .success();
    cmd(&dir)
        .args(["issue", "list", "--state", "closed"])
        .assert()
        .success()
        .stdout(predicates::str::contains("No closed issues."));
}

#[test]
fn list_closed_shows_after_edit() {
    let dir = setup_repo();
    cmd(&dir)
        .args(["issue", "new", "Close me", "--body", ""])
        .assert()
        .success();
    cmd(&dir)
        .args(["issue", "close", "1"])
        .assert()
        .success();
    let out = cmd(&dir)
        .args(["issue", "list", "--state", "closed"])
        .assert()
        .success();
    let stdout = String::from_utf8(out.get_output().stdout.clone()).unwrap();
    assert!(stdout.contains("Close me"));
}

// --- issue show --oneline ---

#[test]
fn status_shows_id_and_title() {
    let dir = setup_repo();
    cmd(&dir)
        .args(["issue", "new", "Status test", "--body", ""])
        .assert()
        .success();
    cmd(&dir)
        .args(["issue", "show", "--oneline", "1"])
        .assert()
        .success()
        .stdout(predicates::str::contains("#1"))
        .stdout(predicates::str::contains("Status test"));
}

#[test]
fn status_missing_issue_exits_nonzero() {
    let dir = setup_repo();
    cmd(&dir)
        .args(["issue", "show", "--oneline", "99"])
        .assert()
        .failure()
        .stderr(predicates::str::contains("not found"));
}

// --- issue show ---

#[test]
fn show_displays_title_and_body() {
    let dir = setup_repo();
    cmd(&dir)
        .args(["issue", "new", "Show me", "--body", "detailed body"])
        .assert()
        .success();
    cmd(&dir)
        .args(["issue", "show", "1"])
        .assert()
        .success()
        .stdout(predicates::str::contains("Show me"))
        .stdout(predicates::str::contains("detailed body"));
}

#[test]
fn show_missing_issue_exits_nonzero() {
    let dir = setup_repo();
    cmd(&dir)
        .args(["issue", "show", "42"])
        .assert()
        .failure()
        .stderr(predicates::str::contains("not found"));
}

// --- issue edit ---

#[test]
fn edit_title() {
    let dir = setup_repo();
    cmd(&dir)
        .args(["issue", "new", "Old title", "--body", ""])
        .assert()
        .success();
    cmd(&dir)
        .args(["issue", "edit", "1", "--title", "New title"])
        .assert()
        .success()
        .stderr(predicates::str::contains("Updated issue #1"));
    cmd(&dir)
        .args(["issue", "show", "1"])
        .assert()
        .success()
        .stdout(predicates::str::contains("New title"));
}

#[test]
fn edit_state_to_closed() {
    let dir = setup_repo();
    cmd(&dir)
        .args(["issue", "new", "To close", "--body", ""])
        .assert()
        .success();
    cmd(&dir)
        .args(["issue", "close", "1"])
        .assert()
        .success();
    cmd(&dir)
        .args(["issue", "show", "--oneline", "1"])
        .assert()
        .success()
        .stdout(predicates::str::contains("closed"));
}

#[test]
fn edit_state_reopen() {
    let dir = setup_repo();
    cmd(&dir)
        .args(["issue", "new", "Reopen me", "--body", ""])
        .assert()
        .success();
    cmd(&dir)
        .args(["issue", "close", "1"])
        .assert()
        .success();
    cmd(&dir)
        .args(["issue", "reopen", "1"])
        .assert()
        .success();
    cmd(&dir)
        .args(["issue", "show", "--oneline", "1"])
        .assert()
        .success()
        .stdout(predicates::str::contains("open"));
}
