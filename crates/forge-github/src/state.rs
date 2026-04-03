//! Sync-state map stored in `refs/forge/sync/github/<owner>/<repo>`.

use anyhow::Result;
use git2::{ErrorCode, ObjectType, Repository};
use std::collections::HashMap;

/// Returns the ref name for the sync state of the given owner/repo.
#[must_use]
pub fn sync_ref_name(owner: &str, repo: &str) -> String {
    format!("refs/forge/sync/github/{owner}/{repo}")
}

/// Load the full sync-state map for `owner`/`repo_name`.
///
/// Keys are of the form `"issues/<n>"`, `"reviews/<n>"`, or `"comments/<id>"`.
/// Values are forge OID strings or chain-entry OID strings.
/// Returns an empty map when the ref does not yet exist.
///
/// # Errors
/// Returns an error if a git operation fails.
pub fn load_sync_state(
    repo: &Repository,
    owner: &str,
    repo_name: &str,
) -> Result<HashMap<String, String>> {
    let ref_name = sync_ref_name(owner, repo_name);
    let reference = match repo.find_reference(&ref_name) {
        Ok(r) => r,
        Err(e) if e.code() == ErrorCode::NotFound => return Ok(HashMap::new()),
        Err(e) => return Err(e.into()),
    };
    let tree = reference.peel_to_commit()?.tree()?;
    read_flat_subtrees(repo, &tree)
}

/// Persist `state` as the new sync-state commit for `owner`/`repo_name`.
///
/// The provided map is written as-is; entries not present in `state` are
/// removed from their subtree. Subtrees not mentioned in `state` at all are
/// preserved from the previous commit.
///
/// # Errors
/// Returns an error if a git operation fails.
#[allow(clippy::implicit_hasher)]
pub fn save_sync_state(
    repo: &Repository,
    owner: &str,
    repo_name: &str,
    state: &HashMap<String, String>,
) -> Result<()> {
    let ref_name = sync_ref_name(owner, repo_name);
    let parent = match repo.find_reference(&ref_name) {
        Ok(r) => Some(r.peel_to_commit()?),
        Err(e) if e.code() == ErrorCode::NotFound => None,
        Err(e) => return Err(e.into()),
    };
    let parent_tree = parent.as_ref().map(git2::Commit::tree).transpose()?;

    // Group entries by sub-tree prefix (e.g. "issues", "reviews", "comments").
    let mut subtrees: HashMap<&str, Vec<(&str, &str)>> = HashMap::new();
    for (key, value) in state {
        if let Some((prefix, leaf)) = key.split_once('/') {
            subtrees
                .entry(prefix)
                .or_default()
                .push((leaf, value.as_str()));
        }
    }

    let mut root_builder = if let Some(ref t) = parent_tree {
        repo.treebuilder(Some(t))?
    } else {
        repo.treebuilder(None)?
    };

    for (prefix, entries) in &subtrees {
        // Each subtree is rebuilt from scratch so the saved state is authoritative
        // for that prefix; entries absent from `state` are dropped.
        //
        // Callers must include **all** entries for every prefix they touch.
        // Any prefix present in `state` will have its subtree replaced entirely;
        // omitting an entry that was previously saved under that prefix deletes it.
        let mut child_builder = repo.treebuilder(None)?;
        for (leaf, value) in entries {
            let blob_oid = repo.blob(value.as_bytes())?;
            child_builder.insert(leaf, blob_oid, 0o100_644)?;
        }
        let child_oid = child_builder.write()?;
        root_builder.insert(prefix, child_oid, 0o040_000)?;
    }

    let tree_oid = root_builder.write()?;
    let tree = repo.find_tree(tree_oid)?;
    let sig = repo.signature()?;
    let parents: Vec<&git2::Commit<'_>> = parent.iter().collect();
    repo.commit(
        Some(&ref_name),
        &sig,
        &sig,
        "forge: update github sync state",
        &tree,
        &parents,
    )?;
    Ok(())
}

/// Look up the forge OID for a GitHub entity by kind and number.
///
/// `kind` is `"issues"`, `"reviews"`, or `"comments"`.
#[allow(clippy::implicit_hasher)]
pub fn lookup_by_github_id<'a>(
    state: &'a HashMap<String, String>,
    kind: &str,
    github_number: u64,
) -> Option<&'a str> {
    state
        .get(&format!("{kind}/{github_number}"))
        .map(String::as_str)
}

/// Reverse-scan the state map to find the GitHub number for a given forge OID.
///
/// TODO: This is O(n) over the entire state map. Consider building a reverse
/// index (`forge_oid → github_id`) for large sync states. When duplicate
/// values exist the result is non-deterministic (whichever entry `HashMap`
/// iteration yields first).
#[must_use]
#[allow(clippy::implicit_hasher)]
pub fn lookup_by_forge_oid(
    state: &HashMap<String, String>,
    kind: &str,
    forge_oid: &str,
) -> Option<u64> {
    let prefix = format!("{kind}/");
    state.iter().find_map(|(key, value)| {
        if key.starts_with(&prefix) && value == forge_oid {
            key[prefix.len()..].parse().ok()
        } else {
            None
        }
    })
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Walk one level of sub-trees and collect all `<subtree>/<leaf>` → value pairs.
fn read_flat_subtrees(repo: &Repository, tree: &git2::Tree<'_>) -> Result<HashMap<String, String>> {
    let mut map = HashMap::new();
    for entry in tree {
        if entry.kind() != Some(ObjectType::Tree) {
            continue;
        }
        let prefix = match entry.name() {
            Some(n) => n.to_string(),
            None => continue,
        };
        let subtree = repo.find_tree(entry.id())?;
        for leaf in &subtree {
            if leaf.kind() == Some(ObjectType::Blob)
                && let Some(name) = leaf.name()
            {
                let blob = repo.find_blob(leaf.id())?;
                let value = match std::str::from_utf8(blob.content()) {
                    Ok(s) => s.to_owned(),
                    Err(e) => {
                        eprintln!(
                            "forge: skipping sync-state entry {prefix}/{name}: \
                             invalid UTF-8: {e}"
                        );
                        continue;
                    }
                };
                map.insert(format!("{prefix}/{name}"), value);
            }
        }
    }
    Ok(map)
}
