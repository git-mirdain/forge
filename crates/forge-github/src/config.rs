//! GitHub sync configuration stored in `refs/forge/config`.

use anyhow::Result;
use git2::{ErrorCode, ObjectType, Repository};

/// Configuration for syncing a single GitHub repository.
pub struct GitHubSyncConfig {
    /// GitHub organization or user name.
    pub owner: String,
    /// GitHub repository name.
    pub repo: String,
    /// Sigil prefix used for cross-references (default `"GH"`).
    pub sigil: String,
    /// Personal access token; falls back to `GITHUB_TOKEN` env var when `None`.
    pub token: Option<String>,
}

const CONFIG_REF: &str = "refs/forge/config";

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
    let sigil = read_blob(repo, &format!("sync/github/{owner}/{repo_name}/sigil"))?
        .unwrap_or_else(|| "GH#".to_string());
    Ok(GitHubSyncConfig {
        owner: owner.to_string(),
        repo: repo_name.to_string(),
        sigil,
        token: None,
    })
}

/// Write `cfg` back to `refs/forge/config`, creating or updating the commit.
///
/// # Errors
/// Returns an error if a git operation fails.
pub fn write_github_config(repo: &Repository, cfg: &GitHubSyncConfig) -> Result<()> {
    let prefix = format!("sync/github/{}/{}", cfg.owner, cfg.repo);
    write_nested_blob(repo, &format!("{prefix}/sigil"), &cfg.sigil)
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn read_blob(repo: &Repository, path: &str) -> Result<Option<String>> {
    let reference = match repo.find_reference(CONFIG_REF) {
        Ok(r) => r,
        Err(e) if e.code() == ErrorCode::NotFound => return Ok(None),
        Err(e) => return Err(e.into()),
    };
    let tree = reference.peel_to_commit()?.tree()?;
    let entry = match tree.get_path(std::path::Path::new(path)) {
        Ok(e) => e,
        Err(e) if e.code() == ErrorCode::NotFound => return Ok(None),
        Err(e) => return Err(e.into()),
    };
    let blob = repo.find_blob(entry.id())?;
    Ok(Some(String::from_utf8_lossy(blob.content()).into_owned()))
}

fn write_nested_blob(repo: &Repository, path: &str, value: &str) -> Result<()> {
    let parts: Vec<&str> = path.split('/').collect();
    let blob_oid = repo.blob(value.as_bytes())?;

    // Load existing root tree (if any).
    let parent = match repo.find_reference(CONFIG_REF) {
        Ok(r) => Some(r.peel_to_commit()?),
        Err(e) if e.code() == ErrorCode::NotFound => None,
        Err(e) => return Err(e.into()),
    };
    let root_tree = parent.as_ref().map(git2::Commit::tree).transpose()?;

    let root_oid = build_tree(repo, root_tree.as_ref(), &parts, blob_oid)?;
    let root = repo.find_tree(root_oid)?;
    let sig = repo.signature()?;
    let parents: Vec<&git2::Commit<'_>> = parent.iter().collect();
    repo.commit(
        Some(CONFIG_REF),
        &sig,
        &sig,
        "forge: update github sync config",
        &root,
        &parents,
    )?;
    Ok(())
}

/// Recursively build a tree, inserting `blob_oid` at `parts[0]/parts[1]/.../leaf`.
fn build_tree(
    repo: &Repository,
    base: Option<&git2::Tree<'_>>,
    parts: &[&str],
    leaf_oid: git2::Oid,
) -> Result<git2::Oid> {
    let mut builder = if let Some(t) = base {
        repo.treebuilder(Some(t))?
    } else {
        repo.treebuilder(None)?
    };

    if parts.len() == 1 {
        builder.insert(parts[0], leaf_oid, 0o100_644)?;
    } else {
        // Descend into sub-tree.
        let child_base: Option<git2::Tree<'_>> = base
            .and_then(|t| t.get_name(parts[0]))
            .filter(|e| e.kind() == Some(ObjectType::Tree))
            .map(|e| repo.find_tree(e.id()))
            .transpose()?;
        let child_oid = build_tree(repo, child_base.as_ref(), &parts[1..], leaf_oid)?;
        builder.insert(parts[0], child_oid, 0o040_000)?;
    }

    Ok(builder.write()?)
}
