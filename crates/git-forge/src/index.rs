use crate::{Error, Result};
use git2::{ErrorCode, ObjectType, Repository};
use std::collections::HashMap;

/// Read all key→value entries from a flat index ref tree.
/// Returns `None` when the ref does not yet exist.
pub(crate) fn read_index(
    repo: &Repository,
    index_ref: &str,
) -> Result<Option<HashMap<String, String>>> {
    let reference = match repo.find_reference(index_ref) {
        Ok(r) => r,
        Err(e) if e.code() == ErrorCode::NotFound => return Ok(None),
        Err(e) => return Err(e.into()),
    };
    let commit = reference.peel_to_commit()?;
    let tree = commit.tree()?;
    let mut map = HashMap::new();
    for entry in &tree {
        if entry.kind() == Some(ObjectType::Blob)
            && let Some(name) = entry.name()
        {
            let blob = repo.find_blob(entry.id())?;
            let value = String::from_utf8_lossy(blob.content()).into_owned();
            map.insert(name.to_string(), value);
        }
    }
    Ok(Some(map))
}

/// Upsert entries into an index ref, creating it if absent.
/// Each entry is a `(key, value)` pair written as a blob named `key`.
pub(crate) fn index_upsert(
    repo: &Repository,
    index_ref: &str,
    entries: &[(&str, &str)],
) -> Result<()> {
    let parent = match repo.find_reference(index_ref) {
        Ok(r) => Some(r.peel_to_commit()?),
        Err(e) if e.code() == ErrorCode::NotFound => None,
        Err(e) => return Err(e.into()),
    };

    let mut builder = if let Some(ref p) = parent {
        repo.treebuilder(Some(&p.tree()?))?
    } else {
        repo.treebuilder(None)?
    };

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
        "forge: update index",
        &tree,
        &parents,
    )?;
    Ok(())
}

/// Resolve a user-supplied `oid_or_id` string to a full 40-char OID string.
///
/// Resolution order:
/// 1. Index alias lookup (e.g. `"GH#1"`, `"auth-bug"`).
/// 2. 40-char hex → exact match in `known_oids`.
/// 3. Shorter hex → OID prefix match in `known_oids`.
///
/// `known_oids` is the list of all entity OIDs (from `Ledger::list`).
pub(crate) fn resolve_oid(
    index: Option<&HashMap<String, String>>,
    known_oids: &[String],
    oid_or_id: &str,
) -> Result<String> {
    let is_hex = |s: &str| s.chars().all(|c| c.is_ascii_hexdigit());

    // Index alias lookup (sigil-prefixed ID, user alias, etc.)
    if let Some(index) = index {
        if let Some(val) = index.get(oid_or_id)
            && val.len() == 40
            && is_hex(val)
        {
            return Ok(val.clone());
        }

        // Retry with leading zeros stripped from the numeric suffix,
        // so `GH#04` resolves to the entry stored as `GH#4`.
        let num_start = oid_or_id.find(|c: char| c.is_ascii_digit());
        if let Some(pos) = num_start {
            let (prefix, num) = oid_or_id.split_at(pos);
            let stripped = format!("{prefix}{}", num.trim_start_matches('0'));
            if stripped != oid_or_id
                && let Some(val) = index.get(&stripped)
                && val.len() == 40
                && is_hex(val)
            {
                return Ok(val.clone());
            }
        }
    }

    // Hex string → match against known OIDs
    if is_hex(oid_or_id) && !oid_or_id.is_empty() {
        // Short pure-numeric hex strings (< 4 chars) are ambiguous with
        // display IDs. If the index didn't match, refuse to fall through
        // to prefix matching — the caller likely meant a display ID.
        if oid_or_id.len() < 4 {
            return Err(Error::NotFound(oid_or_id.to_string()));
        }

        // Exact 40-char match
        if oid_or_id.len() == 40 && known_oids.iter().any(|o| o == oid_or_id) {
            return Ok(oid_or_id.to_string());
        }

        // Prefix match
        let mut matches: Vec<&String> = known_oids
            .iter()
            .filter(|o| o.starts_with(oid_or_id))
            .collect();
        matches.sort();
        return match matches.len() {
            0 => Err(Error::NotFound(oid_or_id.to_string())),
            1 => Ok(matches[0].clone()),
            _ => Err(Error::Ambiguous(oid_or_id.to_string())),
        };
    }

    Err(Error::NotFound(oid_or_id.to_string()))
}

/// Reverse-lookup: find the display ID that maps to a given OID.
pub(crate) fn display_id_for_oid(
    index: Option<&HashMap<String, String>>,
    oid: &str,
) -> Option<String> {
    index?.iter().find_map(
        |(key, val)| {
            if val == oid { Some(key.clone()) } else { None }
        },
    )
}
