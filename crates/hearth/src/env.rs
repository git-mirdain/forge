//! Environment configuration and tree composition.
//!
//! Environments are declared in `env.toml`:
//!
//! ```toml
//! [env.default]
//! trees = ["a3f1c9d...", "b72e4f8..."]
//!
//! [env.dev]
//! extends = "default"
//! trees = ["c91a3b2..."]
//! ```

use std::collections::{BTreeMap, HashMap};
use std::fs;
use std::path::Path;

use git2::{Oid, Repository};
use serde::Deserialize;

use crate::Error;
use crate::store::Store;

/// Top-level configuration loaded from `env.toml`.
#[derive(Debug, Deserialize)]
pub struct Config {
    /// Named environment definitions.
    #[serde(default)]
    pub env: HashMap<String, EnvDef>,
    /// VM configuration (kernel + root filesystem).
    pub vm: Option<VmDef>,
}

/// A single environment definition.
#[derive(Debug, Deserialize)]
pub struct EnvDef {
    /// Name of the environment to extend.
    pub extends: Option<String>,
    /// Component tree hashes in merge order.
    #[serde(default)]
    pub trees: Vec<String>,
    /// Isolation level (0–3).
    pub isolation: Option<u8>,
}

/// VM configuration for levels 2+.
#[derive(Debug, Deserialize)]
pub struct VmDef {
    /// Hash of the vmlinuz blob.
    pub kernel: Option<String>,
    /// Hash of the VM root filesystem tree.
    pub root: Option<String>,
}

/// Load an `env.toml` configuration from disk.
pub fn load_config(path: &Path) -> Result<Config, Error> {
    let content = fs::read_to_string(path).map_err(|e| {
        Error::Config(format!("failed to read {}: {e}", path.display()))
    })?;
    toml::from_str(&content)
        .map_err(|e| Error::Config(format!("failed to parse {}: {e}", path.display())))
}

/// Resolve an environment name to its ordered list of component tree OIDs.
///
/// Follows the `extends` chain, collecting trees from base to derived.
pub fn resolve_trees(config: &Config, name: &str) -> Result<Vec<Oid>, Error> {
    let mut seen = Vec::new();
    let mut chain = Vec::new();
    collect_chain(config, name, &mut seen, &mut chain)?;

    let mut oids = Vec::new();
    for hash_str in &chain {
        let oid = Oid::from_str(hash_str).map_err(|e| {
            Error::Config(format!("invalid tree hash '{hash_str}': {e}"))
        })?;
        oids.push(oid);
    }
    Ok(oids)
}

fn collect_chain(
    config: &Config,
    name: &str,
    seen: &mut Vec<String>,
    out: &mut Vec<String>,
) -> Result<(), Error> {
    if seen.contains(&name.to_string()) {
        return Err(Error::Config(format!(
            "circular extends: {}",
            seen.join(" -> ")
        )));
    }
    seen.push(name.to_string());

    let def = config.env.get(name).ok_or_else(|| {
        Error::Config(format!("environment '{name}' not found in config"))
    })?;

    if let Some(ref base) = def.extends {
        collect_chain(config, base, seen, out)?;
    }

    out.extend(def.trees.iter().cloned());
    Ok(())
}

/// Merge multiple component trees into a single tree.
///
/// Trees are overlaid in order — later entries win on filename conflicts.
/// When two trees both contain a subtree at the same path, their contents
/// are merged recursively.
pub fn merge_trees(store: &Store, tree_oids: &[Oid]) -> Result<Oid, Error> {
    let repo = store.repo();
    let mut merged: BTreeMap<String, MergedEntry> = BTreeMap::new();

    for &oid in tree_oids {
        let tree = repo.find_tree(oid)?;
        overlay_tree(&mut merged, repo, &tree)?;
    }

    write_merged_map(repo, &merged)
}

/// Resolve a named environment fully: resolve its trees, merge them, store
/// the env ref, and return the merged tree hash.
pub fn resolve_env(store: &Store, config: &Config, name: &str) -> Result<Oid, Error> {
    let tree_oids = resolve_trees(config, name)?;
    if tree_oids.is_empty() {
        return Err(Error::Config(format!(
            "environment '{name}' has no component trees"
        )));
    }
    let merged = merge_trees(store, &tree_oids)?;
    store.create_env_ref(merged)?;
    Ok(merged)
}

// ---------------------------------------------------------------------------
// Overlay internals
// ---------------------------------------------------------------------------

enum MergedEntry {
    Blob { oid: Oid, mode: i32 },
    Tree(BTreeMap<String, MergedEntry>),
}

fn overlay_tree(
    base: &mut BTreeMap<String, MergedEntry>,
    repo: &Repository,
    tree: &git2::Tree<'_>,
) -> Result<(), Error> {
    for entry in tree.iter() {
        let name = entry
            .name()
            .ok_or_else(|| Error::Config("non-UTF-8 tree entry name".into()))?
            .to_string();

        if entry.kind() == Some(git2::ObjectType::Tree) {
            let subtree = repo.find_tree(entry.id())?;
            if let Some(MergedEntry::Tree(existing)) = base.get_mut(&name) {
                overlay_tree(existing, repo, &subtree)?;
            } else {
                let mut sub_map = BTreeMap::new();
                overlay_tree(&mut sub_map, repo, &subtree)?;
                base.insert(name, MergedEntry::Tree(sub_map));
            }
        } else {
            base.insert(
                name,
                MergedEntry::Blob {
                    oid: entry.id(),
                    mode: entry.filemode(),
                },
            );
        }
    }
    Ok(())
}

fn write_merged_map(
    repo: &Repository,
    map: &BTreeMap<String, MergedEntry>,
) -> Result<Oid, Error> {
    let mut builder = repo.treebuilder(None)?;
    for (name, entry) in map {
        match entry {
            MergedEntry::Blob { oid, mode } => {
                builder.insert(name, *oid, *mode)?;
            }
            MergedEntry::Tree(sub) => {
                let subtree_oid = write_merged_map(repo, sub)?;
                builder.insert(name, subtree_oid, 0o040_000)?;
            }
        }
    }
    Ok(builder.write()?)
}
