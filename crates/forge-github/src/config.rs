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
/// The sigil subtree is rebuilt from scratch so that sigils removed from `cfg`
/// are also removed from the config ref.
///
/// # Errors
/// Returns an error if a git operation fails.
pub fn write_github_config(repo: &Repository, cfg: &GitHubSyncConfig) -> Result<()> {
    let parent = match repo.find_reference(refs::CONFIG) {
        Ok(r) => Some(r.peel_to_commit()?),
        Err(e) if e.code() == ErrorCode::NotFound => None,
        Err(e) => return Err(e.into()),
    };
    let parent_tree = parent.as_ref().map(git2::Commit::tree).transpose()?;

    // Build the sigil subtree from scratch — drops stale entries.
    let sigil_tree_oid = {
        let mut builder = repo.treebuilder(None)?;
        for (entity, sigil) in &cfg.sigils {
            let blob_oid = repo.blob(sigil.as_bytes())?;
            builder.insert(entity, blob_oid, 0o100_644)?;
        }
        builder.write()?
    };

    // Insert the sigil tree at provider/github/<owner>/<repo>/sigil.
    // This is the same walk-down/fold-up as `build_tree` but the leaf is a
    // tree (mode 0o040_000) instead of a blob.
    let segments: Vec<&str> = vec!["provider", "github", &cfg.owner, &cfg.repo, "sigil"];

    let mut pairs: Vec<(&str, Option<git2::Oid>)> = Vec::new();
    let mut current = parent_tree.as_ref().map(git2::Tree::id);
    for seg in &segments {
        pairs.push((seg, current));
        current = current.and_then(|oid| {
            repo.find_tree(oid)
                .ok()?
                .get_name(seg)
                .filter(|e| e.kind() == Some(ObjectType::Tree))
                .map(|e| e.id())
        });
    }

    let mut child_oid = sigil_tree_oid;
    let mode = 0o040_000; // every level including the leaf is a tree
    for (seg, tree_oid) in pairs.into_iter().rev() {
        let mut builder = match tree_oid {
            Some(oid) => repo.treebuilder(Some(&repo.find_tree(oid)?))?,
            None => repo.treebuilder(None)?,
        };
        builder.insert(seg, child_oid, mode)?;
        child_oid = builder.write()?;
    }

    let root = repo.find_tree(child_oid)?;
    let sig = repo.signature()?;
    let parents: Vec<&git2::Commit<'_>> = parent.iter().collect();
    repo.commit(
        Some(refs::CONFIG),
        &sig,
        &sig,
        "forge: update config",
        &root,
        &parents,
    )?;
    Ok(())
}
