//! GitHub sync configuration stored in `refs/forge/config`.

use std::collections::{BTreeMap, BTreeSet};

use anyhow::Result;
use git_forge::refs;
use git2::{ErrorCode, ObjectType, Repository};

/// Which object types to sync.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum SyncScope {
    /// Sync issues (and their comments) only.
    Issues,
    /// Sync reviews (and their comments) only.
    Reviews,
}

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
    /// Which object types to sync. Defaults to `{Issues}`.
    pub sync: BTreeSet<SyncScope>,
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

    let sync_path = format!("provider/github/{owner}/{repo_name}/sync");
    let sync_map = refs::read_config_subtree(repo, &sync_path)?;
    let sync = if sync_map.is_empty() {
        BTreeSet::from([SyncScope::Issues])
    } else {
        let mut scopes = BTreeSet::new();
        if sync_map.contains_key("issues") {
            scopes.insert(SyncScope::Issues);
        }
        if sync_map.contains_key("reviews") {
            scopes.insert(SyncScope::Reviews);
        }
        scopes
    };

    Ok(GitHubSyncConfig {
        owner: owner.to_string(),
        repo: repo_name.to_string(),
        sigils,
        token: None,
        sync,
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

    // Build the sync subtree.
    let sync_tree_oid = {
        let mut builder = repo.treebuilder(None)?;
        let enabled = repo.blob(b"true")?;
        for scope in &cfg.sync {
            let name = match scope {
                SyncScope::Issues => "issues",
                SyncScope::Reviews => "reviews",
            };
            builder.insert(name, enabled, 0o100_644)?;
        }
        builder.write()?
    };

    // Build the repo-level tree with both sigil and sync subtrees.
    let repo_subtree_oid = {
        let existing = parent_tree.as_ref().and_then(|t| {
            let path = format!("provider/github/{}/{}", cfg.owner, cfg.repo);
            t.get_path(std::path::Path::new(&path))
                .ok()
                .and_then(|e| repo.find_tree(e.id()).ok())
        });
        let mut builder = match existing.as_ref() {
            Some(tree) => repo.treebuilder(Some(tree))?,
            None => repo.treebuilder(None)?,
        };
        builder.insert("sigil", sigil_tree_oid, 0o040_000)?;
        builder.insert("sync", sync_tree_oid, 0o040_000)?;
        builder.write()?
    };

    // Walk up from provider/github/<owner>/<repo> to root.
    let segments: Vec<&str> = vec!["provider", "github", &cfg.owner, &cfg.repo];

    let mut pairs: Vec<(&str, Option<git2::Oid>)> = Vec::new();
    let mut current = parent_tree.as_ref().map(git2::Tree::id);
    for seg in &segments[..segments.len() - 1] {
        pairs.push((seg, current));
        current = current.and_then(|oid| {
            repo.find_tree(oid)
                .ok()?
                .get_name(seg)
                .filter(|e| e.kind() == Some(ObjectType::Tree))
                .map(|e| e.id())
        });
    }

    let mut child_oid = repo_subtree_oid;
    let mode = 0o040_000;
    // Insert the repo tree at the <repo> level, then fold up.
    {
        let mut builder = match current {
            Some(oid) => repo.treebuilder(Some(&repo.find_tree(oid)?))?,
            None => repo.treebuilder(None)?,
        };
        builder.insert(&cfg.repo, child_oid, mode)?;
        child_oid = builder.write()?;
    }
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
