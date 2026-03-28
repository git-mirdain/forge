//! Ref prefix constants and tree helpers for the forge namespace.

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

/// Recursively build a tree, inserting `blob_oid` at `parts[0]/parts[1]/.../leaf`.
///
/// # Errors
/// Returns an error if a git operation fails.
pub fn build_tree(
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
