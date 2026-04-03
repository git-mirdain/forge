//! Integration tests for `forge_github::config` discovery and round-trip.
#![allow(clippy::must_use_candidate, clippy::missing_panics_doc, missing_docs)]

use forge_github::config::{
    GitHubSyncConfig, SyncScope, discover_github_configs, read_github_config, write_github_config,
};
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

fn config_with_sigils(owner: &str, repo_name: &str, sigils: &[(&str, &str)]) -> GitHubSyncConfig {
    GitHubSyncConfig {
        owner: owner.into(),
        repo: repo_name.into(),
        sigils: sigils
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect(),
        token: None,
        sync: vec![],
    }
}

// ---------------------------------------------------------------------------
// discover_github_configs — empty / nonexistent
// ---------------------------------------------------------------------------

#[test]
fn discover_empty_repo_returns_empty() {
    let (_dir, repo) = test_repo();
    let configs = discover_github_configs(&repo).unwrap();
    assert!(configs.is_empty());
}

// ---------------------------------------------------------------------------
// write_github_config / read_github_config round-trip
// ---------------------------------------------------------------------------

#[test]
fn write_then_read_sigils() {
    let (_dir, repo) = test_repo();
    let cfg = config_with_sigils("my-org", "my-repo", &[("issue", "GH#"), ("review", "PR#")]);
    write_github_config(&repo, &cfg).unwrap();

    let loaded = read_github_config(&repo, "my-org", "my-repo").unwrap();
    assert_eq!(loaded.owner, "my-org");
    assert_eq!(loaded.repo, "my-repo");
    assert_eq!(loaded.sigils.get("issue").map(String::as_str), Some("GH#"));
    assert_eq!(loaded.sigils.get("review").map(String::as_str), Some("PR#"));
}

#[test]
fn read_config_no_sigils_returns_empty_map() {
    let (_dir, repo) = test_repo();
    // No write at all — config ref doesn't exist.
    let loaded = read_github_config(&repo, "org", "repo").unwrap();
    assert_eq!(loaded.owner, "org");
    assert_eq!(loaded.repo, "repo");
    assert!(loaded.sigils.is_empty());
}

#[test]
fn write_config_overwrites_sigil() {
    let (_dir, repo) = test_repo();
    let cfg1 = config_with_sigils("org", "repo", &[("issue", "OLD#")]);
    write_github_config(&repo, &cfg1).unwrap();

    let cfg2 = config_with_sigils("org", "repo", &[("issue", "NEW#")]);
    write_github_config(&repo, &cfg2).unwrap();

    let loaded = read_github_config(&repo, "org", "repo").unwrap();
    assert_eq!(loaded.sigils.get("issue").map(String::as_str), Some("NEW#"));
}

#[test]
fn write_config_removes_stale_sigil() {
    let (_dir, repo) = test_repo();
    let cfg1 = config_with_sigils("org", "repo", &[("issue", "GH#"), ("review", "PR#")]);
    write_github_config(&repo, &cfg1).unwrap();

    // Rewrite with only the issue sigil — review should be removed.
    let cfg2 = config_with_sigils("org", "repo", &[("issue", "GH#")]);
    write_github_config(&repo, &cfg2).unwrap();

    let loaded = read_github_config(&repo, "org", "repo").unwrap();
    assert_eq!(loaded.sigils.len(), 1);
    assert_eq!(loaded.sigils.get("issue").map(String::as_str), Some("GH#"));
    assert!(!loaded.sigils.contains_key("review"));
}

// ---------------------------------------------------------------------------
// discover_github_configs
// ---------------------------------------------------------------------------

#[test]
fn discover_finds_written_config() {
    let (_dir, repo) = test_repo();
    let cfg = config_with_sigils("my-org", "my-repo", &[("issue", "GH#")]);
    write_github_config(&repo, &cfg).unwrap();

    let discovered = discover_github_configs(&repo).unwrap();
    assert_eq!(discovered.len(), 1);
    assert_eq!(discovered[0].owner, "my-org");
    assert_eq!(discovered[0].repo, "my-repo");
    assert_eq!(
        discovered[0].sigils.get("issue").map(String::as_str),
        Some("GH#")
    );
}

#[test]
fn discover_finds_multiple_configs() {
    let (_dir, repo) = test_repo();
    write_github_config(
        &repo,
        &config_with_sigils("org", "repo-a", &[("issue", "A#")]),
    )
    .unwrap();
    write_github_config(
        &repo,
        &config_with_sigils("org", "repo-b", &[("issue", "B#")]),
    )
    .unwrap();
    write_github_config(
        &repo,
        &config_with_sigils("other-org", "repo-c", &[("issue", "C#")]),
    )
    .unwrap();

    let discovered = discover_github_configs(&repo).unwrap();
    assert_eq!(discovered.len(), 3);

    let mut pairs: Vec<(&str, &str)> = discovered
        .iter()
        .map(|c| (c.owner.as_str(), c.repo.as_str()))
        .collect();
    pairs.sort_unstable();
    assert_eq!(
        pairs,
        vec![
            ("org", "repo-a"),
            ("org", "repo-b"),
            ("other-org", "repo-c")
        ]
    );
}

#[test]
fn discover_config_sigils_are_correct_per_repo() {
    let (_dir, repo) = test_repo();
    write_github_config(
        &repo,
        &config_with_sigils("org", "repo-a", &[("issue", "AA#")]),
    )
    .unwrap();
    write_github_config(
        &repo,
        &config_with_sigils("org", "repo-b", &[("issue", "BB#")]),
    )
    .unwrap();

    let discovered = discover_github_configs(&repo).unwrap();
    assert_eq!(discovered.len(), 2);

    for cfg in &discovered {
        let expected_sigil = if cfg.repo == "repo-a" { "AA#" } else { "BB#" };
        assert_eq!(
            cfg.sigils.get("issue").map(String::as_str),
            Some(expected_sigil),
            "repo {} had wrong sigil",
            cfg.repo
        );
    }
}

// ---------------------------------------------------------------------------
// sync scope round-trip
// ---------------------------------------------------------------------------

#[test]
fn write_then_read_sync_scopes() {
    let (_dir, repo) = test_repo();
    let mut cfg = config_with_sigils("org", "repo", &[]);
    cfg.sync = vec![SyncScope::Issues, SyncScope::Reviews];
    write_github_config(&repo, &cfg).unwrap();

    let loaded = read_github_config(&repo, "org", "repo").unwrap();
    assert_eq!(loaded.sync, vec![SyncScope::Issues, SyncScope::Reviews]);
}

#[test]
fn sync_scope_defaults_to_issues_when_absent() {
    let (_dir, repo) = test_repo();
    // Write config without sync scope (empty vec → no sync subtree entries).
    let cfg = config_with_sigils("org", "repo", &[("issue", "GH#")]);
    write_github_config(&repo, &cfg).unwrap();

    let loaded = read_github_config(&repo, "org", "repo").unwrap();
    // Empty sync subtree should default to [Issues].
    assert_eq!(loaded.sync, vec![SyncScope::Issues]);
}
