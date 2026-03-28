//! GitHub sync configuration stored in `refs/forge/config`.

use std::collections::BTreeMap;

use anyhow::Result;
use git_forge::refs;
use git2::{ErrorCode, ObjectType, Repository};

/// Configuration for syncing a single GitHub repository.
pub struct GitHubSyncConfig {
    /// GitHub organization or user name.
    pub owner: String,
    /// GitHub repository name.
    pub repo: String,
    /// Entity kind → sigil prefix (e.g. `"issue"` → `"GH#"`).
    pub sigils: BTreeMap<String, String>,
    /// Personal access token; falls back to `GH_TOKEN` env var when `None`.
    pub token: Option<String>,
}

/// Discover all GitHub sync configurations under `refs/forge/config`.
///
/// Walks the `provider/github/<owner>/<repo>/` subtree and returns a config for
/// each `(owner, repo)` pair found. Returns an empty vec when the config ref
/// does not exist.
///
/// # Errors
/// Returns an error if a git operation fails.
pub fn discover_github_configs(repo: &Repository) -> Result<Vec<GitHubSyncConfig>> {
    let reference = match repo.find_reference(refs::CONFIG) {
        Ok(r) => r,
        Err(e) if e.code() == ErrorCode::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(e.into()),
    };
    let root_tree = reference.peel_to_commit()?.tree()?;

    let github_entry = match root_tree.get_path(std::path::Path::new("provider/github")) {
        Ok(e) => e,
        Err(e) if e.code() == ErrorCode::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(e.into()),
    };
    let github_tree = repo.find_tree(github_entry.id())?;

    let mut configs = Vec::new();
    for owner_entry in &github_tree {
        if owner_entry.kind() != Some(ObjectType::Tree) {
            continue;
        }
        let Some(owner) = owner_entry.name() else {
            continue;
        };
        let owner_tree = repo.find_tree(owner_entry.id())?;
        for repo_entry in &owner_tree {
            if repo_entry.kind() != Some(ObjectType::Tree) {
                continue;
            }
            let Some(repo_name) = repo_entry.name() else {
                continue;
            };
            configs.push(read_github_config(repo, owner, repo_name)?);
        }
    }
    Ok(configs)
}

/// Read `GitHubSyncConfig` for the given `owner`/`repo_name` from `refs/forge/config`.
/// Missing blobs resolve to their defaults.
///
/// # Errors
/// Returns an error if a git operation fails.
pub fn read_github_config(
    repo: &Repository,
    owner: &str,
    repo_name: &str,
) -> Result<GitHubSyncConfig> {
    let sigil_path = format!("provider/github/{owner}/{repo_name}/sigil");
    let sigils = refs::read_config_subtree(repo, &sigil_path)?;
    Ok(GitHubSyncConfig {
        owner: owner.to_string(),
        repo: repo_name.to_string(),
        sigils,
        token: None,
    })
}

/// Write `cfg` back to `refs/forge/config`, creating or updating the commit.
///
/// # Errors
/// Returns an error if a git operation fails.
pub fn write_github_config(repo: &Repository, cfg: &GitHubSyncConfig) -> Result<()> {
    let prefix = format!("provider/github/{}/{}", cfg.owner, cfg.repo);
    for (entity, sigil) in &cfg.sigils {
        refs::write_config_blob(repo, &format!("{prefix}/sigil/{entity}"), sigil)?;
    }
    Ok(())
}
