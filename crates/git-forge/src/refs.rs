//! Ref prefix constants and tree helpers for the forge namespace.

use std::collections::BTreeMap;

use git2::{ErrorCode, ObjectType, Repository};

use crate::Result;

/// Entity ref prefix for issues.
pub const ISSUE_PREFIX: &str = "refs/forge/issue/";
/// Entity ref prefix for reviews.
pub const REVIEW_PREFIX: &str = "refs/forge/review/";
/// Chain ref prefix for issue comments.
pub const ISSUE_COMMENTS_PREFIX: &str = "refs/forge/comments/issue/";
/// Chain ref prefix for review comments.
pub const REVIEW_COMMENTS_PREFIX: &str = "refs/forge/comments/review/";
/// Chain ref prefix for standalone object comments.
pub const OBJECT_COMMENTS_PREFIX: &str = "refs/forge/comments/object/";
/// Ref prefix for per-thread comment chains (v2).
pub const COMMENTS_PREFIX: &str = "refs/forge/comments/";
/// Index ref mapping object OIDs to comment thread UUIDs.
pub const COMMENTS_INDEX: &str = "refs/forge/index/comments-by-object";
/// Index ref mapping display IDs ↔ OIDs for issues.
pub const ISSUE_INDEX: &str = "refs/forge/meta/index/issues";
/// Index ref mapping display IDs ↔ OIDs for reviews.
pub const REVIEW_INDEX: &str = "refs/forge/meta/index/reviews";
/// Configuration ref for provider settings.
pub const CONFIG: &str = "refs/forge/config";

/// Read a UTF-8 blob at `path` from the tree pointed to by `config_ref`.
///
/// Returns `Ok(None)` when the ref or any path segment does not exist.
///
/// # Errors
/// Returns an error if a git operation fails.
pub fn read_config_blob(repo: &Repository, path: &str) -> Result<Option<String>> {
    let reference = match repo.find_reference(CONFIG) {
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

/// Write a UTF-8 blob at a nested `path` (slash-separated) under `refs/forge/config`,
/// creating or updating the commit.
///
/// # Errors
/// Returns an error if a git operation fails.
pub fn write_config_blob(repo: &Repository, path: &str, value: &str) -> Result<()> {
    let parts: Vec<&str> = path.split('/').collect();
    let blob_oid = repo.blob(value.as_bytes())?;

    let parent = match repo.find_reference(CONFIG) {
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
        Some(CONFIG),
        &sig,
        &sig,
        "forge: update config",
        &root,
        &parents,
    )?;
    Ok(())
}

/// Read all blob entries from a subtree at `path` under `refs/forge/config`.
///
/// Returns a map of entry name → UTF-8 content. Returns an empty map when the
/// ref or any path segment does not exist.
///
/// # Errors
/// Returns an error if a git operation fails.
pub fn read_config_subtree(repo: &Repository, path: &str) -> Result<BTreeMap<String, String>> {
    let reference = match repo.find_reference(CONFIG) {
        Ok(r) => r,
        Err(e) if e.code() == ErrorCode::NotFound => return Ok(BTreeMap::new()),
        Err(e) => return Err(e.into()),
    };
    let root_tree = reference.peel_to_commit()?.tree()?;
    let subtree_entry = match root_tree.get_path(std::path::Path::new(path)) {
        Ok(e) => e,
        Err(e) if e.code() == ErrorCode::NotFound => return Ok(BTreeMap::new()),
        Err(e) => return Err(e.into()),
    };
    let subtree = repo.find_tree(subtree_entry.id())?;
    let mut map = BTreeMap::new();
    for entry in &subtree {
        if entry.kind() != Some(ObjectType::Blob) {
            continue;
        }
        let Some(name) = entry.name() else {
            continue;
        };
        let blob = repo.find_blob(entry.id())?;
        map.insert(
            name.to_string(),
            String::from_utf8_lossy(blob.content()).into_owned(),
        );
    }
    Ok(map)
}

/// Recursively walk a git tree, collecting `(path, blob_oid)` pairs.
pub fn walk_tree(
    repo: &Repository,
    tree: &git2::Tree<'_>,
    prefix: &str,
    out: &mut Vec<(String, String)>,
) {
    for entry in tree {
        let name = entry.name().unwrap_or("");
        let path = if prefix.is_empty() {
            name.to_string()
        } else {
            format!("{prefix}/{name}")
        };
        match entry.kind() {
            Some(ObjectType::Blob) => {
                out.push((path, entry.id().to_string()));
            }
            Some(ObjectType::Tree) => {
                if let Ok(subtree) = repo.find_tree(entry.id()) {
                    walk_tree(repo, &subtree, &path, out);
                }
            }
            _ => {}
        }
    }
}

/// Build a tree, inserting `leaf_oid` as a blob at `parts[0]/parts[1]/.../leaf`.
///
/// # Errors
/// Returns an error if a git operation fails.
pub fn build_tree(
    repo: &Repository,
    base: Option<&git2::Tree<'_>>,
    parts: &[&str],
    leaf_oid: git2::Oid,
) -> Result<git2::Oid> {
    // Walk down, collecting (segment, existing_tree_oid) pairs.
    let mut pairs: Vec<(&str, Option<git2::Oid>)> = Vec::new();
    let mut current = base.map(git2::Tree::id);
    for &seg in parts {
        pairs.push((seg, current));
        current = match current {
            Some(oid) => repo
                .find_tree(oid)?
                .get_name(seg)
                .filter(|e| e.kind() == Some(ObjectType::Tree))
                .map(|e| e.id()),
            None => None,
        };
    }

    // Fold bottom-up: insert leaf blob, then wrap each parent.
    let mut child_oid = leaf_oid;
    let mut mode = 0o100_644; // first insertion is a blob
    for (seg, tree_oid) in pairs.into_iter().rev() {
        let mut builder = match tree_oid {
            Some(oid) => repo.treebuilder(Some(&repo.find_tree(oid)?))?,
            None => repo.treebuilder(None)?,
        };
        builder.insert(seg, child_oid, mode)?;
        child_oid = builder.write()?;
        mode = 0o040_000; // subsequent insertions are trees
    }
    Ok(child_oid)
}
