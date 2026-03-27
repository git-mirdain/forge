use anyhow::{Result, anyhow};
use git2::{ErrorCode, ObjectType, Repository};
use std::collections::HashMap;

/// Read all key→value entries from a flat index ref tree.
/// Returns an empty map when the ref does not yet exist.
pub(crate) fn read_index(repo: &Repository, index_ref: &str) -> Result<HashMap<String, String>> {
    let reference = match repo.find_reference(index_ref) {
        Ok(r) => r,
        Err(e) if e.code() == ErrorCode::NotFound => return Ok(HashMap::new()),
        Err(e) => return Err(e.into()),
    };
    let commit = reference.peel_to_commit()?;
    let tree = commit.tree()?;
    let mut map = HashMap::new();
    for entry in &tree {
        if entry.kind() == Some(ObjectType::Blob)
            && let Some(name) = entry.name() {
                let blob = repo.find_blob(entry.id())?;
                let value = String::from_utf8_lossy(blob.content()).into_owned();
                map.insert(name.to_string(), value);
            }
    }
    Ok(map)
}

/// Upsert entries into an index ref, creating it if absent.
/// Each entry is a `(key, value)` pair written as a blob named `key`.
pub(crate) fn index_upsert(repo: &Repository, index_ref: &str, entries: &[(&str, &str)]) -> Result<()> {
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
    repo.commit(Some(index_ref), &sig, &sig, "update index", &tree, &parents)?;
    Ok(())
}

/// Resolve a user-supplied `oid_or_id` string to a full 40-char OID string.
///
/// Resolution order:
/// 1. All-digit string → display ID lookup.
/// 2. 40-char hex → exact OID key lookup.
/// 3. Shorter hex string → OID prefix match.
/// 4. Sigil-prefixed ID (e.g. `"GH1"`) or alias → direct key lookup.
pub(crate) fn resolve_oid(index: &HashMap<String, String>, oid_or_id: &str) -> Result<String> {
    let is_hex = |s: &str| s.chars().all(|c| c.is_ascii_hexdigit());

    // All digits → display ID
    if !oid_or_id.is_empty() && oid_or_id.chars().all(|c| c.is_ascii_digit()) {
        return index
            .get(oid_or_id)
            .cloned()
            .ok_or_else(|| anyhow!("no entity with display ID #{oid_or_id}"));
    }

    // 40-char hex → exact OID key
    if oid_or_id.len() == 40 && is_hex(oid_or_id) && index.contains_key(oid_or_id) {
        return Ok(oid_or_id.to_string());
    }

    // Shorter hex → prefix match on OID keys
    if is_hex(oid_or_id) {
        let mut matches: Vec<&String> = index
            .keys()
            .filter(|k| k.len() == 40 && k.starts_with(oid_or_id))
            .collect();
        matches.sort();
        return match matches.len() {
            0 => Err(anyhow!("no entity matching #{oid_or_id}")),
            1 => Ok(matches[0].clone()),
            _ => Err(anyhow!("ambiguous OID prefix #{oid_or_id}")),
        };
    }

    // Sigil-prefixed ID or alias
    if let Some(val) = index.get(oid_or_id) {
        if val.len() == 40 && is_hex(val) {
            return Ok(val.clone());
        }
        // val is itself a display ID
        if let Some(oid) = index.get(val.as_str()) {
            return Ok(oid.clone());
        }
    }

    Err(anyhow!("no entity matching #{oid_or_id}"))
}
