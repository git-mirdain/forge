//! Contributor model backed by `refs/forge/contributors/<uuid-v7>`.
//!
//! Each contributor ref points to a commit whose tree has the layout:
//!
//! ```text
//! ├── handle          # blob: mutable display name, must be unique
//! ├── names/
//! │   └── <display name>  # empty blob — presence means name is active
//! ├── emails/
//! │   └── <address>       # empty blob — presence means address is active
//! ├── keys/
//! │   └── <fingerprint>   # blob: public key material
//! └── roles/
//!     └── <role>          # empty blob — presence means role is granted
//! ```
//!
//! Timestamps live in the commit metadata.

use std::collections::HashMap;

use facet::Facet;
use git2::{ErrorCode, ObjectType, Repository};
use uuid::Uuid;

use crate::refs::CONTRIBUTORS_PREFIX;
use crate::{Error, Result, Store};

// ── subtree helpers ─────────────────────────────────────────────────────────

/// Validate that `key` is safe to use as a single git tree entry name.
fn validate_entry_name(key: &str) -> Result<()> {
    if key.is_empty() || key.contains('/') || key.contains('\0') {
        return Err(Error::Config(format!(
            "invalid entry name {key:?}: must be non-empty with no slashes or NUL bytes"
        )));
    }
    Ok(())
}

/// Insert `blob_oid` as `subtree_name/key` into `root_tree`, returning the
/// new root tree OID.  Creates the subtree if it doesn't exist yet.
fn insert_into_subtree(
    repo: &Repository,
    root_tree: &git2::Tree<'_>,
    subtree_name: &str,
    key: &str,
    blob_oid: git2::Oid,
) -> Result<git2::Oid> {
    validate_entry_name(key)?;
    let existing = root_tree
        .get_name(subtree_name)
        .filter(|e| e.kind() == Some(ObjectType::Tree))
        .and_then(|e| repo.find_tree(e.id()).ok());
    let mut sub_builder = repo.treebuilder(existing.as_ref())?;
    sub_builder.insert(key, blob_oid, 0o100_644)?;
    let new_sub_oid = sub_builder.write()?;
    let mut root_builder = repo.treebuilder(Some(root_tree))?;
    root_builder.insert(subtree_name, new_sub_oid, 0o040_000)?;
    Ok(root_builder.write()?)
}

/// Remove a single entry from a subtree within `root_tree`, returning the new
/// root tree OID.
fn drop_from_subtree(
    repo: &Repository,
    root_tree: &git2::Tree<'_>,
    subtree_name: &str,
    key: &str,
) -> Result<git2::Oid> {
    let entry = root_tree
        .get_name(subtree_name)
        .filter(|e| e.kind() == Some(ObjectType::Tree))
        .ok_or_else(|| Error::NotFound(format!("{subtree_name}/{key}")))?;
    let subtree = repo.find_tree(entry.id())?;
    if subtree.get_name(key).is_none() {
        return Err(Error::NotFound(format!("{subtree_name}/{key}")));
    }
    let mut sub_builder = repo.treebuilder(Some(&subtree))?;
    sub_builder.remove(key)?;
    let new_sub_oid = sub_builder.write()?;
    let mut root_builder = repo.treebuilder(Some(root_tree))?;
    root_builder.insert(subtree_name, new_sub_oid, 0o040_000)?;
    Ok(root_builder.write()?)
}

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
    /// Active display names (e.g. `"Alice Smith"`, `"A. Smith"`).
    pub names: Vec<String>,
    /// Active email addresses for identity matching.
    pub emails: Vec<String>,
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

    let names = read_subtree_keys(repo, &tree, "names");
    let emails = read_subtree_keys(repo, &tree, "emails");
    let keys = read_subtree_keys(repo, &tree, "keys");
    let roles = read_subtree_keys(repo, &tree, "roles");

    Ok(Some(Contributor {
        id: id.clone(),
        handle,
        names,
        emails,
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

/// Fields for creating a new contributor.
struct NewContributor<'a> {
    handle: &'a Handle,
    names: &'a [&'a str],
    emails: &'a [&'a str],
    keys: &'a [(&'a str, &'a [u8])],
    roles: &'a [&'a str],
}

/// Write a new contributor tree to the repo as the initial commit on its ref.
fn write_contributor(
    repo: &Repository,
    id: &ContributorId,
    fields: &NewContributor<'_>,
    message: &str,
) -> Result<()> {
    let empty = repo.blob(b"")?;
    let mut builder = repo.treebuilder(None)?;

    let handle_oid = repo.blob(fields.handle.as_str().as_bytes())?;
    builder.insert("handle", handle_oid, 0o100_644)?;

    if !fields.names.is_empty() {
        let mut nb = repo.treebuilder(None)?;
        for name in fields.names {
            nb.insert(name, empty, 0o100_644)?;
        }
        builder.insert("names", nb.write()?, 0o040_000)?;
    }

    if !fields.emails.is_empty() {
        let mut eb = repo.treebuilder(None)?;
        for email in fields.emails {
            eb.insert(email, empty, 0o100_644)?;
        }
        builder.insert("emails", eb.write()?, 0o040_000)?;
    }

    if !fields.keys.is_empty() {
        let mut kb = repo.treebuilder(None)?;
        for (fp, material) in fields.keys {
            let blob_oid = repo.blob(material)?;
            kb.insert(fp, blob_oid, 0o100_644)?;
        }
        builder.insert("keys", kb.write()?, 0o040_000)?;
    }

    if !fields.roles.is_empty() {
        let mut rb = repo.treebuilder(None)?;
        for role in fields.roles {
            rb.insert(role, empty, 0o100_644)?;
        }
        builder.insert("roles", rb.write()?, 0o040_000)?;
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
    pub fn create_contributor(
        &self,
        handle: &str,
        names: &[&str],
        emails: &[&str],
        roles: &[&str],
    ) -> Result<Contributor> {
        let handle = Handle::new(handle)?;
        if self.find_contributor_by_handle(&handle)?.is_some() {
            return Err(Error::Config(format!("handle already taken: {handle}")));
        }
        let id = ContributorId::new();
        write_contributor(
            self.repo,
            &id,
            &NewContributor {
                handle: &handle,
                names,
                emails,
                keys: &[],
                roles,
            },
            "create contributor",
        )?;
        Ok(Contributor {
            id,
            handle,
            names: names.iter().map(ToString::to_string).collect(),
            emails: emails.iter().map(ToString::to_string).collect(),
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
    //
    // TODO: O(n) full table scan — should use a handle → UUID index.
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
    /// contributors' `emails/` subtrees.
    ///
    /// # Errors
    /// Returns an error if a git operation fails.
    pub fn email_to_contributor_map(&self) -> Result<HashMap<String, ContributorId>> {
        let mut map = HashMap::new();
        for c in self.list_contributors()? {
            for email in &c.emails {
                map.insert(email.clone(), c.id.clone());
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
            names: contributor.names,
            emails: contributor.emails,
            keys: contributor.keys,
            roles: contributor.roles,
        })
    }

    /// Add a role to a contributor.
    ///
    /// # Errors
    /// Returns [`Error::NotFound`] if the contributor doesn't exist.
    //
    // TODO: contributor mutations (add/remove role, name, email, key) each
    // create a separate commit. A batch of edits can leave partial state if
    // an intermediate operation fails. These should be consolidated into a
    // single commit per logical edit session.
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
        let new_tree_oid = insert_into_subtree(self.repo, &old_tree, "roles", role, empty_oid)?;
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
            names: contributor.names,
            emails: contributor.emails,
            keys: contributor.keys,
            roles,
        })
    }

    /// Remove a role from a contributor.
    ///
    /// # Errors
    /// Returns [`Error::NotFound`] if the contributor or role doesn't exist.
    pub fn remove_contributor_role(&self, handle: &str, role: &str) -> Result<Contributor> {
        let h = Handle::new(handle)?;
        let contributor = self
            .find_contributor_by_handle(&h)?
            .ok_or_else(|| Error::NotFound(handle.to_string()))?;

        let ref_name = contributor_ref(&contributor.id);
        let reference = self.repo.find_reference(&ref_name)?;
        let parent_commit = reference.peel_to_commit()?;
        let old_tree = parent_commit.tree()?;

        let new_tree_oid = drop_from_subtree(self.repo, &old_tree, "roles", role)?;
        let new_tree = self.repo.find_tree(new_tree_oid)?;

        let sig = self.repo.signature()?;
        self.repo.commit(
            Some(&ref_name),
            &sig,
            &sig,
            "remove contributor role",
            &new_tree,
            &[&parent_commit],
        )?;

        let mut roles = contributor.roles;
        roles.retain(|r| r != role);
        Ok(Contributor {
            id: contributor.id,
            handle: h,
            names: contributor.names,
            emails: contributor.emails,
            keys: contributor.keys,
            roles,
        })
    }

    /// Add a display name to a contributor.
    ///
    /// # Errors
    /// Returns [`Error::NotFound`] if the contributor doesn't exist.
    pub fn add_contributor_name(&self, handle: &str, name: &str) -> Result<Contributor> {
        let h = Handle::new(handle)?;
        let contributor = self
            .find_contributor_by_handle(&h)?
            .ok_or_else(|| Error::NotFound(handle.to_string()))?;

        let ref_name = contributor_ref(&contributor.id);
        let reference = self.repo.find_reference(&ref_name)?;
        let parent_commit = reference.peel_to_commit()?;
        let old_tree = parent_commit.tree()?;

        let empty_oid = self.repo.blob(b"")?;
        let new_tree_oid = insert_into_subtree(self.repo, &old_tree, "names", name, empty_oid)?;
        let new_tree = self.repo.find_tree(new_tree_oid)?;

        let sig = self.repo.signature()?;
        self.repo.commit(
            Some(&ref_name),
            &sig,
            &sig,
            "add contributor name",
            &new_tree,
            &[&parent_commit],
        )?;

        let mut names = contributor.names;
        if !names.contains(&name.to_string()) {
            names.push(name.to_string());
        }
        Ok(Contributor {
            id: contributor.id,
            handle: h,
            names,
            emails: contributor.emails,
            keys: contributor.keys,
            roles: contributor.roles,
        })
    }

    /// Remove a display name from a contributor.
    ///
    /// # Errors
    /// Returns [`Error::NotFound`] if the contributor or name doesn't exist.
    pub fn remove_contributor_name(&self, handle: &str, name: &str) -> Result<Contributor> {
        let h = Handle::new(handle)?;
        let contributor = self
            .find_contributor_by_handle(&h)?
            .ok_or_else(|| Error::NotFound(handle.to_string()))?;

        let ref_name = contributor_ref(&contributor.id);
        let reference = self.repo.find_reference(&ref_name)?;
        let parent_commit = reference.peel_to_commit()?;
        let old_tree = parent_commit.tree()?;

        let new_tree_oid = drop_from_subtree(self.repo, &old_tree, "names", name)?;
        let new_tree = self.repo.find_tree(new_tree_oid)?;

        let sig = self.repo.signature()?;
        self.repo.commit(
            Some(&ref_name),
            &sig,
            &sig,
            "remove contributor name",
            &new_tree,
            &[&parent_commit],
        )?;

        let mut names = contributor.names;
        names.retain(|n| n != name);
        Ok(Contributor {
            id: contributor.id,
            handle: h,
            names,
            emails: contributor.emails,
            keys: contributor.keys,
            roles: contributor.roles,
        })
    }

    /// Add an email address to a contributor.
    ///
    /// # Errors
    /// Returns [`Error::NotFound`] if the contributor doesn't exist.
    pub fn add_contributor_email(&self, handle: &str, email: &str) -> Result<Contributor> {
        let h = Handle::new(handle)?;
        let contributor = self
            .find_contributor_by_handle(&h)?
            .ok_or_else(|| Error::NotFound(handle.to_string()))?;

        let ref_name = contributor_ref(&contributor.id);
        let reference = self.repo.find_reference(&ref_name)?;
        let parent_commit = reference.peel_to_commit()?;
        let old_tree = parent_commit.tree()?;

        let empty_oid = self.repo.blob(b"")?;
        let new_tree_oid = insert_into_subtree(self.repo, &old_tree, "emails", email, empty_oid)?;
        let new_tree = self.repo.find_tree(new_tree_oid)?;

        let sig = self.repo.signature()?;
        self.repo.commit(
            Some(&ref_name),
            &sig,
            &sig,
            "add contributor email",
            &new_tree,
            &[&parent_commit],
        )?;

        let mut emails = contributor.emails;
        if !emails.contains(&email.to_string()) {
            emails.push(email.to_string());
        }
        Ok(Contributor {
            id: contributor.id,
            handle: h,
            names: contributor.names,
            emails,
            keys: contributor.keys,
            roles: contributor.roles,
        })
    }

    /// Remove an email address from a contributor.
    ///
    /// # Errors
    /// Returns [`Error::NotFound`] if the contributor or email doesn't exist.
    pub fn remove_contributor_email(&self, handle: &str, email: &str) -> Result<Contributor> {
        let h = Handle::new(handle)?;
        let contributor = self
            .find_contributor_by_handle(&h)?
            .ok_or_else(|| Error::NotFound(handle.to_string()))?;

        let ref_name = contributor_ref(&contributor.id);
        let reference = self.repo.find_reference(&ref_name)?;
        let parent_commit = reference.peel_to_commit()?;
        let old_tree = parent_commit.tree()?;

        let new_tree_oid = drop_from_subtree(self.repo, &old_tree, "emails", email)?;
        let new_tree = self.repo.find_tree(new_tree_oid)?;

        let sig = self.repo.signature()?;
        self.repo.commit(
            Some(&ref_name),
            &sig,
            &sig,
            "remove contributor email",
            &new_tree,
            &[&parent_commit],
        )?;

        let mut emails = contributor.emails;
        emails.retain(|e| e != email);
        Ok(Contributor {
            id: contributor.id,
            handle: h,
            names: contributor.names,
            emails,
            keys: contributor.keys,
            roles: contributor.roles,
        })
    }

    /// Add a public key to a contributor.
    ///
    /// # Errors
    /// Returns [`Error::NotFound`] if the contributor doesn't exist.
    pub fn add_contributor_key(
        &self,
        handle: &str,
        fingerprint: &str,
        material: &[u8],
    ) -> Result<Contributor> {
        let h = Handle::new(handle)?;
        let contributor = self
            .find_contributor_by_handle(&h)?
            .ok_or_else(|| Error::NotFound(handle.to_string()))?;

        let ref_name = contributor_ref(&contributor.id);
        let reference = self.repo.find_reference(&ref_name)?;
        let parent_commit = reference.peel_to_commit()?;
        let old_tree = parent_commit.tree()?;

        let blob_oid = self.repo.blob(material)?;
        let new_tree_oid =
            insert_into_subtree(self.repo, &old_tree, "keys", fingerprint, blob_oid)?;
        let new_tree = self.repo.find_tree(new_tree_oid)?;

        let sig = self.repo.signature()?;
        self.repo.commit(
            Some(&ref_name),
            &sig,
            &sig,
            "add contributor key",
            &new_tree,
            &[&parent_commit],
        )?;

        let mut keys = contributor.keys;
        if !keys.contains(&fingerprint.to_string()) {
            keys.push(fingerprint.to_string());
        }
        Ok(Contributor {
            id: contributor.id,
            handle: h,
            names: contributor.names,
            emails: contributor.emails,
            keys,
            roles: contributor.roles,
        })
    }

    /// Remove a public key from a contributor.
    ///
    /// # Errors
    /// Returns [`Error::NotFound`] if the contributor or key doesn't exist.
    pub fn remove_contributor_key(&self, handle: &str, fingerprint: &str) -> Result<Contributor> {
        let h = Handle::new(handle)?;
        let contributor = self
            .find_contributor_by_handle(&h)?
            .ok_or_else(|| Error::NotFound(handle.to_string()))?;

        let ref_name = contributor_ref(&contributor.id);
        let reference = self.repo.find_reference(&ref_name)?;
        let parent_commit = reference.peel_to_commit()?;
        let old_tree = parent_commit.tree()?;

        let new_tree_oid = drop_from_subtree(self.repo, &old_tree, "keys", fingerprint)?;
        let new_tree = self.repo.find_tree(new_tree_oid)?;

        let sig = self.repo.signature()?;
        self.repo.commit(
            Some(&ref_name),
            &sig,
            &sig,
            "remove contributor key",
            &new_tree,
            &[&parent_commit],
        )?;

        let mut keys = contributor.keys;
        keys.retain(|k| k != fingerprint);
        Ok(Contributor {
            id: contributor.id,
            handle: h,
            names: contributor.names,
            emails: contributor.emails,
            keys,
            roles: contributor.roles,
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
        let email = cfg
            .get_string("user.email")
            .map_err(|_| Error::Config("user.email not set".into()))?;
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
        self.create_contributor(&handle, &[name.as_str()], &[email.as_str()], &["admin"])
    }
}
