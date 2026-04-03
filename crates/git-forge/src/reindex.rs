//! Shared helpers for rebuilding display-ID indexes.

use std::collections::HashMap;

use git_ledger::LedgerEntry;
use git2::Repository;

use crate::Result;
use crate::refs;

/// Build a map from `(owner, repo)` → sigil for the given entity kind.
///
/// Reads every `provider/github/{owner}/{repo}/sigil/{entity_kind}` blob
/// from `refs/forge/config`.
pub fn build_sigil_map(
    repo: &Repository,
    entity_kind: &str,
) -> Result<HashMap<(String, String), String>> {
    let configs = list_github_configs(repo)?;
    let mut map = HashMap::new();
    for (owner, repo_name) in configs {
        let path = format!("provider/github/{owner}/{repo_name}/sigil");
        let sigils = refs::read_config_subtree(repo, &path)?;
        if let Some(sigil) = sigils.get(entity_kind) {
            map.insert((owner, repo_name), sigil.clone());
        }
    }
    Ok(map)
}

/// Extract a display ID from a ledger entry's `source/url` field.
///
/// Parses `https://github.com/{owner}/{repo}/{url_kind}/{number}` and looks up
/// the sigil for `(owner, repo)`. `url_kind` is `"issues"` or `"pull"`.
//
// TODO: hardcoded to GitHub URL patterns — non-GitHub providers (GitLab,
// Gitea, etc.) are silently skipped during reindex.
pub fn display_id_from_source(
    entry: &LedgerEntry,
    sigil_map: &HashMap<(String, String), String>,
    url_kind: &str,
) -> Option<String> {
    let source_url = entry
        .fields
        .iter()
        .find(|(k, _)| k == "source/url")
        .map(|(_, v)| String::from_utf8_lossy(v).into_owned())?;

    // Parse: https://github.com/{owner}/{repo}/{kind}/{number}
    let path = source_url.strip_prefix("https://github.com/")?;
    let parts: Vec<&str> = path.splitn(4, '/').collect();
    if parts.len() < 4 || parts[2] != url_kind {
        return None;
    }
    let owner = parts[0];
    let repo = parts[1];
    let number = parts[3];

    let key = (owner.to_string(), repo.to_string());
    let sigil = sigil_map.get(&key)?;
    Some(format!("{sigil}{number}"))
}

/// Write an index ref from scratch (not appending — replaces entire tree).
pub fn write_index_from_scratch(
    repo: &Repository,
    index_ref: &str,
    entries: &[(&str, &str)],
) -> Result<()> {
    let parent = match repo.find_reference(index_ref) {
        Ok(r) => Some(r.peel_to_commit()?),
        Err(e) if e.code() == git2::ErrorCode::NotFound => None,
        Err(e) => return Err(e.into()),
    };

    // Build a fresh tree (no parent tree — full replacement).
    let mut builder = repo.treebuilder(None)?;
    for (key, value) in entries {
        let blob_oid = repo.blob(value.as_bytes())?;
        builder.insert(key, blob_oid, 0o100_644)?;
    }

    let tree_oid = builder.write()?;
    let tree = repo.find_tree(tree_oid)?;
    let sig = repo.signature()?;
    let parents: Vec<&git2::Commit<'_>> = parent.iter().collect();
    repo.commit(
        Some(index_ref),
        &sig,
        &sig,
        "forge: reindex",
        &tree,
        &parents,
    )?;
    Ok(())
}

/// List all `(owner, repo)` pairs under `provider/github/` in the config.
fn list_github_configs(repo: &Repository) -> Result<Vec<(String, String)>> {
    use git2::{ErrorCode, ObjectType};

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

    let mut pairs = Vec::new();
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
            pairs.push((owner.to_string(), repo_name.to_string()));
        }
    }
    Ok(pairs)
}
