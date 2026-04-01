//! Contributor model backed by `refs/forge/contributors/<uuid-v7>`.
//!
//! Each contributor ref points to a commit whose tree has the layout:
//!
//! ```text
//! ├── handle          # blob: mutable display name, must be unique
//! ├── keys/
//! │   └── <fingerprint>  # blob: public key material
//! └── roles/
//!     └── <role>     # empty blob — presence means role is granted
//! ```
//!
//! Authorship and timestamps live in the commit metadata.

use std::collections::HashMap;

use facet::Facet;
use git2::{ErrorCode, ObjectType, Repository};
use uuid::Uuid;

use crate::refs::{CONTRIBUTORS_PREFIX, build_tree};
use crate::{Error, Result, Store};

// ── newtype wrappers ─────────────────────────────────────────────────────────

/// Opaque UUID v7 identifier for a contributor.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Facet)]
pub struct ContributorId(String);

impl ContributorId {
    /// Generate a fresh UUID v7 contributor ID.
    #[must_use]
    pub fn new() -> Self {
        Self(Uuid::now_v7().to_string())
    }

    /// Wrap an existing UUID string without re-generating.
    ///
    /// # Errors
    /// Returns an error if `s` is not a valid UUID.
    pub fn parse(s: &str) -> Result<Self> {
        Uuid::parse_str(s).map_err(|_| Error::Config(format!("invalid contributor id: {s}")))?;
        Ok(Self(s.to_string()))
    }

    /// Return the inner UUID string.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl Default for ContributorId {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Display for ContributorId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// A validated contributor handle (no slashes, no whitespace).
#[derive(Debug, Clone, PartialEq, Eq, Facet)]
pub struct Handle(String);

impl Handle {
    /// Parse and validate a handle.
    ///
    /// # Errors
    /// Returns an error if the handle is empty, contains `/`, or contains
    /// ASCII whitespace.
    pub fn new(s: &str) -> Result<Self> {
        if s.is_empty() || s.contains('/') || s.chars().any(|c| c.is_ascii_whitespace()) {
            return Err(Error::Config(format!(
                "invalid handle {s:?}: must be non-empty with no slashes or whitespace"
            )));
        }
        Ok(Self(s.to_string()))
    }

    /// Return the inner string.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for Handle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

// ── entity ───────────────────────────────────────────────────────────────────

/// A contributor stored at `refs/forge/contributors/<uuid-v7>`.
#[derive(Debug, Clone, Facet)]
pub struct Contributor {
    /// Stable UUID v7 identity.
    pub id: ContributorId,
    /// Mutable display handle.
    pub handle: Handle,
    /// Public key fingerprints present in `keys/`.
    pub keys: Vec<String>,
    /// Roles present in `roles/` (e.g. `"admin"`, `"maintainer"`).
    pub roles: Vec<String>,
}

// ── internal helpers ─────────────────────────────────────────────────────────

fn contributor_ref(id: &ContributorId) -> String {
    format!("{CONTRIBUTORS_PREFIX}{}", id.as_str())
}

/// Read a contributor from its ref, returning `None` if the ref is missing.
fn read_contributor(repo: &Repository, id: &ContributorId) -> Result<Option<Contributor>> {
    let ref_name = contributor_ref(id);
    let reference = match repo.find_reference(&ref_name) {
        Ok(r) => r,
        Err(e) if e.code() == ErrorCode::NotFound => return Ok(None),
        Err(e) => return Err(e.into()),
    };
    let tree = reference.peel_to_commit()?.tree()?;

    let handle_blob = tree
        .get_name("handle")
        .and_then(|e| repo.find_blob(e.id()).ok())
        .map(|b| String::from_utf8_lossy(b.content()).into_owned())
        .unwrap_or_default();

    let handle = Handle::new(&handle_blob)?;

    let keys = read_subtree_keys(repo, &tree, "keys");
    let roles = read_subtree_keys(repo, &tree, "roles");

    Ok(Some(Contributor {
        id: id.clone(),
        handle,
        keys,
        roles,
    }))
}

/// Collect the entry names under `subtree_name` in `tree`.
fn read_subtree_keys(repo: &Repository, tree: &git2::Tree<'_>, subtree_name: &str) -> Vec<String> {
    let Some(entry) = tree.get_name(subtree_name) else {
        return Vec::new();
    };
    if entry.kind() != Some(ObjectType::Tree) {
        return Vec::new();
    }
    let Ok(subtree) = repo.find_tree(entry.id()) else {
        return Vec::new();
    };
    subtree
        .iter()
        .filter_map(|e| e.name().map(String::from))
        .collect()
}

/// Write a new contributor tree to the repo as the initial commit on its ref.
fn write_contributor(
    repo: &Repository,
    id: &ContributorId,
    handle: &Handle,
    keys: &[(&str, &[u8])],
    roles: &[&str],
    message: &str,
) -> Result<()> {
    let mut builder = repo.treebuilder(None)?;

    let handle_oid = repo.blob(handle.as_str().as_bytes())?;
    builder.insert("handle", handle_oid, 0o100_644)?;

    if !keys.is_empty() {
        let mut kb = repo.treebuilder(None)?;
        for (fp, material) in keys {
            let blob_oid = repo.blob(material)?;
            kb.insert(fp, blob_oid, 0o100_644)?;
        }
        let keys_oid = kb.write()?;
        builder.insert("keys", keys_oid, 0o040_000)?;
    }

    if !roles.is_empty() {
        let empty = repo.blob(b"")?;
        let mut rb = repo.treebuilder(None)?;
        for role in roles {
            rb.insert(role, empty, 0o100_644)?;
        }
        let roles_oid = rb.write()?;
        builder.insert("roles", roles_oid, 0o040_000)?;
    }

    let tree_oid = builder.write()?;
    let tree = repo.find_tree(tree_oid)?;
    let sig = repo.signature()?;
    let ref_name = contributor_ref(id);
    repo.commit(Some(&ref_name), &sig, &sig, message, &tree, &[])?;
    Ok(())
}

// ── Store impl ────────────────────────────────────────────────────────────────

impl Store<'_> {
    /// Create a new contributor.
    ///
    /// The new contributor gets a fresh UUID v7 identity.
    ///
    /// # Errors
    /// Returns an error if the handle is invalid, already taken, or a git
    /// operation fails.
    pub fn create_contributor(&self, handle: &str, roles: &[&str]) -> Result<Contributor> {
        let handle = Handle::new(handle)?;
        if self.find_contributor_by_handle(&handle)?.is_some() {
            return Err(Error::Config(format!("handle already taken: {handle}")));
        }
        let id = ContributorId::new();
        write_contributor(self.repo, &id, &handle, &[], roles, "create contributor")?;
        Ok(Contributor {
            id,
            handle,
            keys: Vec::new(),
            roles: roles.iter().map(ToString::to_string).collect(),
        })
    }

    /// Fetch a contributor by UUID string.
    ///
    /// # Errors
    /// Returns [`Error::NotFound`] if no matching contributor exists.
    pub fn get_contributor(&self, id_str: &str) -> Result<Contributor> {
        let id = ContributorId::parse(id_str)?;
        read_contributor(self.repo, &id)?.ok_or_else(|| Error::NotFound(id_str.to_string()))
    }

    /// Resolve a handle to a contributor.
    ///
    /// Scans all contributor refs. Returns `None` if no match.
    ///
    /// # Errors
    /// Returns an error if a git operation fails.
    pub fn find_contributor_by_handle(&self, handle: &Handle) -> Result<Option<Contributor>> {
        for c in self.list_contributors()? {
            if c.handle == *handle {
                return Ok(Some(c));
            }
        }
        Ok(None)
    }

    /// Resolve a handle string to a contributor UUID.
    ///
    /// # Errors
    /// Returns [`Error::NotFound`] if no matching contributor exists.
    pub fn resolve_handle(&self, handle: &str) -> Result<ContributorId> {
        let h = Handle::new(handle)?;
        self.find_contributor_by_handle(&h)?
            .map(|c| c.id)
            .ok_or_else(|| Error::NotFound(handle.to_string()))
    }

    /// Resolve a commit signing key fingerprint to a contributor UUID.
    ///
    /// Scans `keys/` subtrees across all contributors.
    ///
    /// # Errors
    /// Returns an error if a git operation fails.
    pub fn resolve_fingerprint(&self, fingerprint: &str) -> Result<Option<ContributorId>> {
        for c in self.list_contributors()? {
            if c.keys.iter().any(|k| k == fingerprint) {
                return Ok(Some(c.id));
            }
        }
        Ok(None)
    }

    /// Build a map from email address → contributor UUID by scanning all
    /// contributors' commit author emails.
    ///
    /// This walks every contributor ref and reads the commit author's email.
    ///
    /// # Errors
    /// Returns an error if a git operation fails.
    pub fn email_to_contributor_map(&self) -> Result<HashMap<String, ContributorId>> {
        let mut map = HashMap::new();
        let ids = self.list_contributor_ids();
        for id in ids {
            let ref_name = contributor_ref(&id);
            if let Ok(reference) = self.repo.find_reference(&ref_name)
                && let Ok(commit) = reference.peel_to_commit()
                && let Some(email) = commit.author().email()
            {
                map.insert(email.to_string(), id);
            }
        }
        Ok(map)
    }

    /// List all contributors.
    ///
    /// # Errors
    /// Returns an error if a git operation fails.
    pub fn list_contributors(&self) -> Result<Vec<Contributor>> {
        let ids = self.list_contributor_ids();
        let mut out = Vec::new();
        for id in ids {
            if let Some(c) = read_contributor(self.repo, &id)? {
                out.push(c);
            }
        }
        Ok(out)
    }

    /// List contributor UUIDs by scanning refs under [`CONTRIBUTORS_PREFIX`].
    fn list_contributor_ids(&self) -> Vec<ContributorId> {
        let mut ids = Vec::new();
        if let Ok(refs) = self
            .repo
            .references_glob(&format!("{CONTRIBUTORS_PREFIX}*"))
        {
            for r in refs.flatten() {
                if let Some(name) = r.name()
                    && let Some(suffix) = name.strip_prefix(CONTRIBUTORS_PREFIX)
                    && let Ok(id) = ContributorId::parse(suffix)
                {
                    ids.push(id);
                }
            }
        }
        ids
    }

    /// Rename a contributor's handle.
    ///
    /// # Errors
    /// Returns [`Error::NotFound`] if `old_handle` doesn't match any contributor,
    /// or an error if `new_handle` is already taken.
    pub fn rename_contributor(&self, old_handle: &str, new_handle: &str) -> Result<Contributor> {
        let old = Handle::new(old_handle)?;
        let new = Handle::new(new_handle)?;

        let contributor = self
            .find_contributor_by_handle(&old)?
            .ok_or_else(|| Error::NotFound(old_handle.to_string()))?;

        if self.find_contributor_by_handle(&new)?.is_some() {
            return Err(Error::Config(format!("handle already taken: {new}")));
        }

        let ref_name = contributor_ref(&contributor.id);
        let reference = self.repo.find_reference(&ref_name)?;
        let parent_commit = reference.peel_to_commit()?;
        let old_tree = parent_commit.tree()?;

        // Rebuild tree with updated handle blob.
        let mut builder = self.repo.treebuilder(Some(&old_tree))?;
        let handle_oid = self.repo.blob(new.as_str().as_bytes())?;
        builder.insert("handle", handle_oid, 0o100_644)?;
        let new_tree_oid = builder.write()?;
        let new_tree = self.repo.find_tree(new_tree_oid)?;

        let sig = self.repo.signature()?;
        self.repo.commit(
            Some(&ref_name),
            &sig,
            &sig,
            "rename contributor",
            &new_tree,
            &[&parent_commit],
        )?;

        Ok(Contributor {
            id: contributor.id,
            handle: new,
            keys: contributor.keys,
            roles: contributor.roles,
        })
    }

    /// Add a role to a contributor.
    ///
    /// # Errors
    /// Returns [`Error::NotFound`] if the contributor doesn't exist.
    pub fn add_contributor_role(&self, handle: &str, role: &str) -> Result<Contributor> {
        let h = Handle::new(handle)?;
        let contributor = self
            .find_contributor_by_handle(&h)?
            .ok_or_else(|| Error::NotFound(handle.to_string()))?;

        let ref_name = contributor_ref(&contributor.id);
        let reference = self.repo.find_reference(&ref_name)?;
        let parent_commit = reference.peel_to_commit()?;
        let old_tree = parent_commit.tree()?;

        let empty_oid = self.repo.blob(b"")?;
        let role_path = format!("roles/{role}");
        let parts: Vec<&str> = role_path.split('/').collect();
        let new_tree_oid = build_tree(self.repo, Some(&old_tree), &parts, empty_oid)?;
        let new_tree = self.repo.find_tree(new_tree_oid)?;

        let sig = self.repo.signature()?;
        self.repo.commit(
            Some(&ref_name),
            &sig,
            &sig,
            "add contributor role",
            &new_tree,
            &[&parent_commit],
        )?;

        let mut roles = contributor.roles;
        if !roles.contains(&role.to_string()) {
            roles.push(role.to_string());
        }
        Ok(Contributor {
            id: contributor.id,
            handle: h,
            keys: contributor.keys,
            roles,
        })
    }

    /// Bootstrap: create the first contributor from the repo's git identity,
    /// assigning the `admin` role.
    ///
    /// No-op if any contributor already exists.
    ///
    /// # Errors
    /// Returns an error if `user.name` is not configured or a git operation
    /// fails.
    pub fn bootstrap_contributor(&self) -> Result<Contributor> {
        if !self.list_contributors()?.is_empty() {
            return Err(Error::Config(
                "bootstrap: contributors already exist".to_string(),
            ));
        }
        let cfg = self.repo.config()?;
        let name = cfg
            .get_string("user.name")
            .map_err(|_| Error::Config("user.name not set".into()))?;
        let handle = name
            .split_whitespace()
            .next()
            .unwrap_or(&name)
            .to_ascii_lowercase();
        let handle = handle.replace(|c: char| !c.is_alphanumeric() && c != '-' && c != '_', "");
        let handle = if handle.is_empty() {
            "admin".to_string()
        } else {
            handle
        };
        self.create_contributor(&handle, &["admin"])
    }
}
