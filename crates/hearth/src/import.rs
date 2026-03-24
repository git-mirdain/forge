//! Import external sources into the content-addressed store.
//!
//! Supported sources:
//! - `dir`: a local directory
//! - `tarball`: a `.tar` or `.tar.gz` archive
//! - `oci`: an OCI image reference (not yet implemented)

use std::collections::BTreeMap;
use std::fs;
use std::io::Read;
use std::path::Path;

use git2::{Oid, Repository};

use crate::Error;
use crate::store::Store;

// ---------------------------------------------------------------------------
// Directory import
// ---------------------------------------------------------------------------

/// Import a local directory into the store, returning the root tree OID.
pub fn import_dir(store: &Store, path: &Path) -> Result<Oid, Error> {
    if !path.is_dir() {
        return Err(Error::Config(format!(
            "not a directory: {}",
            path.display()
        )));
    }
    let oid = write_tree_from_dir(store.repo(), path)?;
    store.create_tree_ref(oid)?;
    Ok(oid)
}

fn write_tree_from_dir(repo: &Repository, dir: &Path) -> Result<Oid, Error> {
    let mut builder = repo.treebuilder(None)?;

    let mut entries: Vec<_> = fs::read_dir(dir)?.collect::<Result<Vec<_>, _>>()?;
    entries.sort_by_key(fs::DirEntry::file_name);

    for entry in entries {
        let ft = entry.file_type()?;
        let name = entry.file_name();
        let name_str = name.to_str().ok_or_else(|| {
            Error::Config(format!(
                "non-UTF-8 filename: {}",
                entry.path().display()
            ))
        })?;

        if ft.is_dir() {
            let subtree_oid = write_tree_from_dir(repo, &entry.path())?;
            builder.insert(name_str, subtree_oid, 0o040_000)?;
        } else if ft.is_symlink() {
            write_symlink_entry(repo, &mut builder, name_str, &entry.path())?;
        } else {
            let content = fs::read(entry.path())?;
            let blob_oid = repo.blob(&content)?;
            let mode = file_mode(&entry.path());
            builder.insert(name_str, blob_oid, mode)?;
        }
    }

    Ok(builder.write()?)
}

#[cfg(unix)]
fn write_symlink_entry(
    repo: &Repository,
    builder: &mut git2::TreeBuilder<'_>,
    name: &str,
    path: &Path,
) -> Result<(), Error> {
    use std::os::unix::ffi::OsStrExt;
    let target = fs::read_link(path)?;
    let blob_oid = repo.blob(target.as_os_str().as_bytes())?;
    builder.insert(name, blob_oid, 0o12_0000)?;
    Ok(())
}

#[cfg(not(unix))]
fn write_symlink_entry(
    _repo: &Repository,
    _builder: &mut git2::TreeBuilder<'_>,
    _name: &str,
    _path: &Path,
) -> Result<(), Error> {
    Ok(())
}

#[cfg(unix)]
fn file_mode(path: &Path) -> i32 {
    use std::os::unix::fs::PermissionsExt;
    match fs::metadata(path) {
        Ok(m) if m.permissions().mode() & 0o111 != 0 => 0o100_755,
        _ => 0o100_644,
    }
}

#[cfg(not(unix))]
fn file_mode(_path: &Path) -> i32 {
    0o100_644
}

// ---------------------------------------------------------------------------
// Tarball import
// ---------------------------------------------------------------------------

/// Import a tarball (`.tar` or `.tar.gz`) into the store.
///
/// `strip_prefix` removes N leading path components from each entry, similar
/// to `tar --strip-components=N`.
pub fn import_tarball(store: &Store, path: &Path, strip_prefix: usize) -> Result<Oid, Error> {
    let file = fs::File::open(path)?;
    let reader: Box<dyn Read> = if is_gzipped(path) {
        Box::new(flate2::read::GzDecoder::new(file))
    } else {
        Box::new(file)
    };

    let mut archive = tar::Archive::new(reader);
    let repo = store.repo();
    let mut root = TreeNode::Dir(BTreeMap::new());

    for entry in archive.entries()? {
        let mut entry = entry?;
        let path = entry.path()?.into_owned();

        if entry.header().entry_type().is_dir() {
            continue;
        }

        let components: Vec<&str> = path
            .components()
            .filter_map(|c| match c {
                std::path::Component::Normal(s) => s.to_str(),
                _ => None,
            })
            .collect();

        if components.len() <= strip_prefix {
            continue;
        }
        let components = &components[strip_prefix..];

        let entry_type = entry.header().entry_type();
        if entry_type.is_symlink() {
            if let Some(target) = entry.link_name()? {
                let target_str = target.to_str().ok_or_else(|| {
                    Error::Config("non-UTF-8 symlink target in tarball".into())
                })?;
                let oid = repo.blob(target_str.as_bytes())?;
                insert_tree_node(&mut root, components, oid, 0o12_0000);
            }
        } else {
            let mut content = Vec::new();
            entry.read_to_end(&mut content)?;
            let oid = repo.blob(&content)?;
            let mode = if entry.header().mode()? & 0o111 != 0 {
                0o100_755
            } else {
                0o100_644
            };
            insert_tree_node(&mut root, components, oid, mode);
        }
    }

    let oid = write_tree_node(repo, root)?;
    store.create_tree_ref(oid)?;
    Ok(oid)
}

fn is_gzipped(path: &Path) -> bool {
    let name = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("");
    name.ends_with(".gz") || name.ends_with(".tgz")
}

// ---------------------------------------------------------------------------
// OCI import (stub)
// ---------------------------------------------------------------------------

/// Import an OCI image into the store.
///
/// Not yet implemented. Returns an error.
pub fn import_oci(_store: &Store, _image_ref: &str) -> Result<Oid, Error> {
    todo!("OCI import")
}

// ---------------------------------------------------------------------------
// In-memory tree builder for tarball import
// ---------------------------------------------------------------------------

enum TreeNode {
    Blob { oid: Oid, mode: i32 },
    Dir(BTreeMap<String, TreeNode>),
}

fn insert_tree_node(root: &mut TreeNode, components: &[&str], oid: Oid, mode: i32) {
    let TreeNode::Dir(children) = root else {
        return;
    };

    if components.len() == 1 {
        children.insert(components[0].to_string(), TreeNode::Blob { oid, mode });
        return;
    }

    let child = children
        .entry(components[0].to_string())
        .or_insert_with(|| TreeNode::Dir(BTreeMap::new()));
    insert_tree_node(child, &components[1..], oid, mode);
}

fn write_tree_node(repo: &Repository, node: TreeNode) -> Result<Oid, Error> {
    let TreeNode::Dir(children) = node else {
        return Err(Error::Config("expected directory node at root".into()));
    };

    let mut builder = repo.treebuilder(None)?;
    for (name, child) in children {
        match child {
            TreeNode::Blob { oid, mode } => {
                builder.insert(&name, oid, mode)?;
            }
            TreeNode::Dir(_) => {
                let subtree_oid = write_tree_node(repo, TreeNode::Dir(unsafe_take(child)))?;
                builder.insert(&name, subtree_oid, 0o040_000)?;
            }
        }
    }
    Ok(builder.write()?)
}

/// Extract the inner `BTreeMap` from a `TreeNode::Dir`.
///
/// # Panics
///
/// Panics if `node` is not `TreeNode::Dir`. The call-site guarantees this
/// via the surrounding `match` arm.
fn unsafe_take(node: TreeNode) -> BTreeMap<String, TreeNode> {
    match node {
        TreeNode::Dir(map) => map,
        TreeNode::Blob { .. } => unreachable!(),
    }
}
