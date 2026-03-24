//! Environment configuration and tree composition.
//!
//! Environments are configured via two files:
//!
//! **`.forge/toolchains.toml`** declares named toolchains with a source URI
//! and a content-addressed tree hash managed by hearth:
//!
//! ```toml
//! [rust]
//! source = "git://kiln-packages/rust@1.82.0"
//! oid = "a3f1c9d..."
//!
//! [python]
//! source = "git://kiln-packages/python@3.12"
//! oid = "b72e4f8..."
//! ```
//!
//! **`.forge/environment.toml`** references toolchains by name and may also
//! list raw tree hashes as an escape hatch:
//!
//! ```toml
//! [project]
//! default = "rust"
//!
//! [env.rust]
//! toolchains = ["rust"]
//! trees = ["c91a3b2..."]
//! extras = ["/usr/bin"]
//!
//! [env.dev]
//! extends = "rust"
//! toolchains = ["python"]
//! trees = ["d82b4c3..."]
//! ```

use std::collections::{BTreeMap, HashMap};
use std::fs;
use std::path::Path;

use git2::{Oid, Repository};
use serde::{Deserialize, Serialize};

use crate::Error;
use crate::store::Store;

/// Project-level configuration.
#[derive(Debug, Deserialize)]
pub struct ProjectDef {
    /// Name of the default environment.
    pub default: String,
}

/// Top-level configuration loaded from `.forge/environment.toml`.
#[derive(Debug, Deserialize)]
pub struct Config {
    /// Project-level settings.
    pub project: ProjectDef,
    /// Named environment definitions.
    #[serde(default)]
    pub env: HashMap<String, EnvDef>,
    /// VM configuration (kernel + root filesystem).
    pub vm: Option<VmDef>,
}

impl Config {
    /// Return the name of the default environment.
    pub fn default_env(&self) -> &str {
        &self.project.default
    }
}

/// A single environment definition.
#[derive(Debug, Deserialize)]
pub struct EnvDef {
    /// Name of the environment to extend.
    pub extends: Option<String>,
    /// Toolchain names from `toolchains.toml` to include.
    #[serde(default)]
    pub toolchains: Vec<String>,
    /// Component tree hashes in merge order.
    #[serde(default)]
    pub trees: Vec<String>,
    /// Host paths added to PATH (not content-addressed).
    #[serde(default)]
    pub extras: Vec<String>,
    /// Isolation level (0–2).
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

/// A single toolchain definition from `toolchains.toml`.
#[derive(Debug, Deserialize, Serialize)]
pub struct ToolchainDef {
    /// Source URI (e.g. `git://kiln-packages/rust@1.82.0`).
    pub source: String,
    /// Resolved content-addressed tree hash (managed by hearth).
    pub oid: Option<String>,
    /// Number of leading path components stripped on import.
    #[serde(default, rename = "strip-prefix", skip_serializing_if = "is_zero")]
    pub strip_prefix: usize,
}

/// Top-level configuration loaded from `.forge/toolchains.toml`.
#[derive(Debug, Default, Deserialize, Serialize)]
pub struct ToolchainsConfig {
    /// Named toolchain definitions.
    #[serde(flatten)]
    pub toolchains: HashMap<String, ToolchainDef>,
}

fn is_zero(v: &usize) -> bool {
    *v == 0
}

/// Load an `env.toml` configuration from disk.
pub fn load_config(path: &Path) -> Result<Config, Error> {
    let content = fs::read_to_string(path)
        .map_err(|e| Error::Config(format!("failed to read {}: {e}", path.display())))?;
    toml::from_str(&content)
        .map_err(|e| Error::Config(format!("failed to parse {}: {e}", path.display())))
}

/// Load a `toolchains.toml` configuration from disk.
pub fn load_toolchains(path: &Path) -> Result<ToolchainsConfig, Error> {
    let content = fs::read_to_string(path)
        .map_err(|e| Error::Config(format!("failed to read {}: {e}", path.display())))?;
    toml::from_str(&content)
        .map_err(|e| Error::Config(format!("failed to parse {}: {e}", path.display())))
}

/// Write a `toolchains.toml` configuration to disk.
pub fn save_toolchains(path: &Path, config: &ToolchainsConfig) -> Result<(), Error> {
    let content = toml::to_string_pretty(config)
        .map_err(|e| Error::Config(format!("failed to serialize toolchains: {e}")))?;
    fs::write(path, content)
        .map_err(|e| Error::Config(format!("failed to write {}: {e}", path.display())))
}

/// Resolve an environment name to its ordered list of component tree OIDs.
///
/// Follows the `extends` chain, collecting toolchain names and trees from
/// base to derived. Toolchain OIDs come first, then raw trees.
pub fn resolve_trees(
    config: &Config,
    toolchains: Option<&ToolchainsConfig>,
    name: &str,
) -> Result<Vec<Oid>, Error> {
    let resolved = resolve_chain(config, name)?;
    let mut oids = Vec::new();

    for tc_name in &resolved.toolchain_names {
        let tc_config = toolchains.ok_or_else(|| {
            Error::Config(format!(
                "environment '{name}' references toolchain '{tc_name}' but no toolchains config provided"
            ))
        })?;
        let tc_def = tc_config.toolchains.get(tc_name).ok_or_else(|| {
            Error::Config(format!(
                "toolchain '{tc_name}' not found in toolchains config"
            ))
        })?;
        let hash_str = tc_def
            .oid
            .as_deref()
            .ok_or_else(|| Error::Config(format!("toolchain '{tc_name}' has no resolved oid")))?;
        let oid = Oid::from_str(hash_str)
            .map_err(|e| Error::Config(format!("invalid oid for toolchain '{tc_name}': {e}")))?;
        oids.push(oid);
    }

    for hash_str in &resolved.trees {
        let oid = Oid::from_str(hash_str)
            .map_err(|e| Error::Config(format!("invalid tree hash '{hash_str}': {e}")))?;
        oids.push(oid);
    }
    Ok(oids)
}

/// Resolve an environment name to its extras (host paths).
///
/// Follows the `extends` chain, collecting extras from base to derived.
pub fn resolve_extras(config: &Config, name: &str) -> Result<Vec<String>, Error> {
    Ok(resolve_chain(config, name)?.extras)
}

struct ResolvedChain {
    toolchain_names: Vec<String>,
    trees: Vec<String>,
    extras: Vec<String>,
}

fn resolve_chain(config: &Config, name: &str) -> Result<ResolvedChain, Error> {
    let mut seen = Vec::new();
    let mut toolchain_names = Vec::new();
    let mut trees = Vec::new();
    let mut extras = Vec::new();
    collect_chain(
        config,
        name,
        &mut seen,
        &mut toolchain_names,
        &mut trees,
        &mut extras,
    )?;
    Ok(ResolvedChain {
        toolchain_names,
        trees,
        extras,
    })
}

fn collect_chain(
    config: &Config,
    name: &str,
    seen: &mut Vec<String>,
    toolchain_names: &mut Vec<String>,
    trees: &mut Vec<String>,
    extras: &mut Vec<String>,
) -> Result<(), Error> {
    if seen.contains(&name.to_string()) {
        return Err(Error::Config(format!(
            "circular extends: {}",
            seen.join(" -> ")
        )));
    }
    seen.push(name.to_string());

    let def = config
        .env
        .get(name)
        .ok_or_else(|| Error::Config(format!("environment '{name}' not found in config")))?;

    if let Some(ref base) = def.extends {
        collect_chain(config, base, seen, toolchain_names, trees, extras)?;
    }

    toolchain_names.extend(def.toolchains.iter().cloned());
    trees.extend(def.trees.iter().cloned());
    extras.extend(def.extras.iter().cloned());
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
pub fn resolve_env(
    store: &Store,
    config: &Config,
    toolchains: Option<&ToolchainsConfig>,
    name: &str,
) -> Result<Oid, Error> {
    let tree_oids = resolve_trees(config, toolchains, name)?;
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

fn write_merged_map(repo: &Repository, map: &BTreeMap<String, MergedEntry>) -> Result<Oid, Error> {
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
