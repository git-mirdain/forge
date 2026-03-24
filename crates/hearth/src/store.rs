//! Content-addressed store backed by a bare Git repository.
//!
//! Layout on disk:
//!
//! ```text
//! <root>/
//!   objects/              Git object store (libgit2 bare repo internals)
//!   refs/                 Git refs (hearth/trees/*, hearth/envs/*, ...)
//!   blobs/<hash>          one checked-out copy of each blob
//!   store/<tree-hash>/    hardlinks into blobs/, structured as a tree
//!   runs/<id>/            per-invocation capture directories
//! ```

use std::fs;
use std::path::{Path, PathBuf};

use git2::{Oid, Repository};

use crate::{ENVS_REF_PREFIX, Error, TREES_REF_PREFIX};

/// A content-addressed store backed by a bare Git repository.
pub struct Store {
    repo: Repository,
    root: PathBuf,
}

impl Store {
    /// Open or initialize a store at the given path.
    ///
    /// Creates the bare Git repository and auxiliary directories (`blobs/`,
    /// `store/`, `runs/`) if they do not already exist.
    pub fn open_or_init(root: &Path) -> Result<Self, Error> {
        fs::create_dir_all(root)?;
        let repo = match Repository::open_bare(root) {
            Ok(repo) => repo,
            Err(_) => Repository::init_bare(root)?,
        };
        for dir in &["blobs", "store", "runs"] {
            fs::create_dir_all(root.join(dir))?;
        }
        Ok(Self {
            repo,
            root: root.to_path_buf(),
        })
    }

    /// Open or initialize the default store at `~/.hearth/`.
    pub fn open_default() -> Result<Self, Error> {
        let home =
            std::env::var("HOME").map_err(|_| Error::Config("HOME not set".into()))?;
        Self::open_or_init(&PathBuf::from(home).join(".hearth"))
    }

    /// Return a reference to the underlying Git repository.
    #[must_use]
    pub fn repo(&self) -> &Repository {
        &self.repo
    }

    /// Return the store root path.
    #[must_use]
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Write a blob to the object store, returning its OID.
    pub fn write_blob(&self, content: &[u8]) -> Result<Oid, Error> {
        Ok(self.repo.blob(content)?)
    }

    /// Create a ref for an imported component tree.
    pub fn create_tree_ref(&self, tree_oid: Oid) -> Result<(), Error> {
        let ref_name = format!("{TREES_REF_PREFIX}{tree_oid}");
        self.repo
            .reference(&ref_name, tree_oid, true, "hearth import")?;
        Ok(())
    }

    /// Create a ref for a merged environment tree.
    pub fn create_env_ref(&self, tree_oid: Oid) -> Result<(), Error> {
        let ref_name = format!("{ENVS_REF_PREFIX}{tree_oid}");
        self.repo
            .reference(&ref_name, tree_oid, true, "hearth env")?;
        Ok(())
    }

    /// List all imported component tree OIDs.
    pub fn list_trees(&self) -> Result<Vec<Oid>, Error> {
        list_refs(&self.repo, TREES_REF_PREFIX)
    }

    /// List all merged environment tree OIDs.
    pub fn list_envs(&self) -> Result<Vec<Oid>, Error> {
        list_refs(&self.repo, ENVS_REF_PREFIX)
    }

    /// Materialize a tree to disk as a hardlink farm.
    ///
    /// Writes each blob to `blobs/<hash>` and creates hardlinks from
    /// `store/<tree-hash>/<path>` into the blob cache. Returns the path to
    /// the materialized tree.
    pub fn materialize(&self, tree_oid: Oid) -> Result<PathBuf, Error> {
        let dest = self.root.join("store").join(tree_oid.to_string());
        if dest.exists() {
            return Ok(dest);
        }
        fs::create_dir_all(&dest)?;
        let tree = self.repo.find_tree(tree_oid)?;
        self.materialize_tree(&tree, &dest)?;
        Ok(dest)
    }

    /// Materialize a tree to an arbitrary destination path.
    pub fn materialize_to(&self, tree_oid: Oid, dest: &Path) -> Result<(), Error> {
        fs::create_dir_all(dest)?;
        let tree = self.repo.find_tree(tree_oid)?;
        self.materialize_tree(&tree, dest)
    }

    fn materialize_tree(&self, tree: &git2::Tree<'_>, dest: &Path) -> Result<(), Error> {
        for entry in tree.iter() {
            let name = entry
                .name()
                .ok_or_else(|| Error::Config("non-UTF-8 tree entry name".into()))?;
            let path = dest.join(name);

            match entry.kind() {
                Some(git2::ObjectType::Tree) => {
                    fs::create_dir_all(&path)?;
                    let subtree = self.repo.find_tree(entry.id())?;
                    self.materialize_tree(&subtree, &path)?;
                }
                Some(git2::ObjectType::Blob) => {
                    if entry.filemode() == 0o12_0000 {
                        self.materialize_symlink(entry.id(), &path)?;
                    } else {
                        self.materialize_blob(entry.id(), &path)?;
                    }
                }
                _ => {}
            }
        }
        Ok(())
    }

    fn materialize_blob(&self, oid: Oid, dest: &Path) -> Result<(), Error> {
        let cache = self.root.join("blobs").join(oid.to_string());
        if !cache.exists() {
            let blob = self.repo.find_blob(oid)?;
            fs::write(&cache, blob.content())?;
        }
        // Hardlink from destination into the blob cache. Fall back to copy
        // if hardlinking fails (e.g. cross-device).
        if fs::hard_link(&cache, dest).is_err() {
            fs::copy(&cache, dest)?;
        }
        Ok(())
    }

    #[cfg(unix)]
    fn materialize_symlink(&self, oid: Oid, dest: &Path) -> Result<(), Error> {
        let blob = self.repo.find_blob(oid)?;
        let target = std::str::from_utf8(blob.content())
            .map_err(|e| Error::Config(format!("symlink target is not UTF-8: {e}")))?;
        std::os::unix::fs::symlink(target, dest)?;
        Ok(())
    }

    #[cfg(not(unix))]
    fn materialize_symlink(&self, _oid: Oid, _dest: &Path) -> Result<(), Error> {
        Err(Error::Config(
            "symlink materialization is not supported on this platform".into(),
        ))
    }

    /// Garbage-collect the blob cache.
    ///
    /// Removes blob cache entries whose hardlink count is 1 — only the cache
    /// entry itself references the inode, so no materialized store uses it.
    /// Returns the number of blobs removed.
    #[cfg(unix)]
    pub fn gc_blobs(&self) -> Result<u64, Error> {
        use std::os::unix::fs::MetadataExt;

        let blobs_dir = self.root.join("blobs");
        if !blobs_dir.exists() {
            return Ok(0);
        }
        let mut removed = 0u64;
        for entry in fs::read_dir(&blobs_dir)? {
            let entry = entry?;
            if entry.metadata()?.nlink() == 1 {
                fs::remove_file(entry.path())?;
                removed += 1;
            }
        }
        Ok(removed)
    }

    /// Garbage-collect the blob cache (non-Unix stub).
    #[cfg(not(unix))]
    pub fn gc_blobs(&self) -> Result<u64, Error> {
        Ok(0)
    }

    /// Remove a materialized store entry.
    pub fn remove_store_entry(&self, tree_oid: Oid) -> Result<(), Error> {
        let path = self.root.join("store").join(tree_oid.to_string());
        if path.exists() {
            fs::remove_dir_all(&path)?;
        }
        Ok(())
    }
}

fn list_refs(repo: &Repository, prefix: &str) -> Result<Vec<Oid>, Error> {
    let mut oids = Vec::new();
    for reference in repo.references_glob(&format!("{prefix}*"))? {
        let reference = reference?;
        if let Some(oid) = reference.target() {
            oids.push(oid);
        }
    }
    Ok(oids)
}
