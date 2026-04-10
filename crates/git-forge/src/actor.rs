//! Actor model backed by `refs/forge/actors/<uuid-v7>`.
//!
//! An actor is any entity — human or non-human — that participates in the
//! commit graph.  The actor's role set determines what they can do:
//! `contributor`, `admin`, `maintainer` for humans; `ci`, `formatter`, etc.
//! for tools.
//!
//! Each actor ref points to a commit whose tree has the layout:
//!
//! ```text
//! ├── handle          # blob: mutable display name, must be unique
//! ├── names/
//! │   └── <name>      # empty blob — any name this actor uses in git fields
//! ├── emails/
//! │   └── <address>   # empty blob — any email this actor uses
//! ├── keys/
//! │   └── <fp>        # blob: public key material (optional)
//! ├── attributes/
//! │   └── <key>       # blob: attribute value, e.g. vendor, model (optional)
//! └── roles/
//!     └── <role>      # empty blob — presence means role is granted
//! ```
//!
//! Timestamps live in the commit metadata.

use std::collections::{BTreeMap, HashMap};

use facet::Facet;
use git2::{ErrorCode, ObjectType, Repository};
use uuid::Uuid;

use crate::refs::ACTORS_PREFIX;
use crate::{Error, Result, Store};

// ── subtree helpers ───────────────────────────────────────────────────────────

pub(crate) fn validate_entry_name(key: &str) -> Result<()> {
    if key.is_empty() || key.contains('/') || key.contains('\0') {
        return Err(Error::Config(format!(
            "invalid entry name {key:?}: must be non-empty with no slashes or NUL bytes"
        )));
    }
    Ok(())
}

pub(crate) fn insert_into_subtree(
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

pub(crate) fn drop_from_subtree(
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

pub(crate) fn read_subtree_keys(
    repo: &Repository,
    tree: &git2::Tree<'_>,
    subtree_name: &str,
) -> Vec<String> {
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

fn read_subtree_kv(
    repo: &Repository,
    tree: &git2::Tree<'_>,
    subtree_name: &str,
) -> BTreeMap<String, String> {
    let Some(entry) = tree.get_name(subtree_name) else {
        return BTreeMap::new();
    };
    if entry.kind() != Some(ObjectType::Tree) {
        return BTreeMap::new();
    }
    let Ok(subtree) = repo.find_tree(entry.id()) else {
        return BTreeMap::new();
    };
    subtree
        .iter()
        .filter_map(|e| {
            let name = e.name()?.to_string();
            let blob = repo.find_blob(e.id()).ok()?;
            let value = String::from_utf8_lossy(blob.content()).into_owned();
            Some((name, value))
        })
        .collect()
}

// ── newtype wrappers ──────────────────────────────────────────────────────────

/// Opaque UUID v7 identifier for an actor.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Facet)]
pub struct ActorId(String);

impl ActorId {
    /// Generate a fresh UUID v7 actor ID.
    #[must_use]
    pub fn new() -> Self {
        Self(Uuid::now_v7().to_string())
    }

    /// Wrap an existing UUID string without re-generating.
    ///
    /// # Errors
    /// Returns an error if `s` is not a valid UUID.
    pub fn parse(s: &str) -> Result<Self> {
        Uuid::parse_str(s).map_err(|_| Error::Config(format!("invalid actor id: {s}")))?;
        Ok(Self(s.to_string()))
    }

    /// Return the inner UUID string.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl Default for ActorId {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Display for ActorId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// A validated handle (no slashes, no whitespace).
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

// ── entity ────────────────────────────────────────────────────────────────────

/// An actor stored at `refs/forge/actors/<uuid-v7>`.
///
/// Actors are humans or tools that participate in the commit graph.
/// The `roles/` subtree determines what they can do — e.g. `contributor`,
/// `admin` for humans; `ci`, `formatter` for tools.
#[derive(Debug, Clone, Facet)]
pub struct Actor {
    /// Stable UUID v7 identity.
    pub id: ActorId,
    /// Mutable display handle.
    pub handle: Handle,
    /// Names this actor is known by (display names and git identity names).
    pub names: Vec<String>,
    /// Email addresses for identity matching.
    pub emails: Vec<String>,
    /// Public key fingerprints present in `keys/`.
    pub keys: Vec<String>,
    /// Arbitrary key→value attributes (e.g. `vendor`, `model`, `type`).
    pub attributes: BTreeMap<String, String>,
    /// Roles present in `roles/`.
    pub roles: Vec<String>,
}

// ── internal helpers ──────────────────────────────────────────────────────────

fn actor_ref(id: &ActorId) -> String {
    format!("{ACTORS_PREFIX}{}", id.as_str())
}

fn read_actor(repo: &Repository, id: &ActorId) -> Result<Option<Actor>> {
    let ref_name = actor_ref(id);
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
    let attributes = read_subtree_kv(repo, &tree, "attributes");
    let roles = read_subtree_keys(repo, &tree, "roles");

    Ok(Some(Actor {
        id: id.clone(),
        handle,
        names,
        emails,
        keys,
        attributes,
        roles,
    }))
}

struct NewActor<'a> {
    handle: &'a Handle,
    names: &'a [&'a str],
    emails: &'a [&'a str],
    keys: &'a [(&'a str, &'a [u8])],
    attributes: &'a [(&'a str, &'a str)],
    roles: &'a [&'a str],
}

fn write_actor(
    repo: &Repository,
    id: &ActorId,
    fields: &NewActor<'_>,
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

    if !fields.attributes.is_empty() {
        let mut atb = repo.treebuilder(None)?;
        for (key, value) in fields.attributes {
            let blob_oid = repo.blob(value.as_bytes())?;
            atb.insert(key, blob_oid, 0o100_644)?;
        }
        builder.insert("attributes", atb.write()?, 0o040_000)?;
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
    let ref_name = actor_ref(id);
    repo.commit(Some(&ref_name), &sig, &sig, message, &tree, &[])?;
    Ok(())
}

// ── Store impl ────────────────────────────────────────────────────────────────

impl Store<'_> {
    /// Create a new actor.
    ///
    /// # Errors
    /// Returns an error if the handle is invalid, already taken, or a git
    /// operation fails.
    pub fn create_actor(
        &self,
        handle: &str,
        names: &[&str],
        emails: &[&str],
        roles: &[&str],
    ) -> Result<Actor> {
        let handle = Handle::new(handle)?;
        if self.find_actor_by_handle(&handle)?.is_some() {
            return Err(Error::Config(format!("handle already taken: {handle}")));
        }
        let id = ActorId::new();
        write_actor(
            self.repo,
            &id,
            &NewActor {
                handle: &handle,
                names,
                emails,
                keys: &[],
                attributes: &[],
                roles,
            },
            "create actor",
        )?;
        Ok(Actor {
            id,
            handle,
            names: names.iter().map(ToString::to_string).collect(),
            emails: emails.iter().map(ToString::to_string).collect(),
            keys: Vec::new(),
            attributes: BTreeMap::new(),
            roles: roles.iter().map(ToString::to_string).collect(),
        })
    }

    /// Fetch an actor by UUID string.
    ///
    /// # Errors
    /// Returns [`Error::NotFound`] if no matching actor exists.
    pub fn get_actor(&self, id_str: &str) -> Result<Actor> {
        let id = ActorId::parse(id_str)?;
        read_actor(self.repo, &id)?.ok_or_else(|| Error::NotFound(id_str.to_string()))
    }

    /// Resolve a handle to an actor.
    ///
    /// # Errors
    /// Returns an error if a git operation fails.
    pub fn find_actor_by_handle(&self, handle: &Handle) -> Result<Option<Actor>> {
        for a in self.list_actors()? {
            if a.handle == *handle {
                return Ok(Some(a));
            }
        }
        Ok(None)
    }

    /// Resolve a handle string to an actor UUID.
    ///
    /// # Errors
    /// Returns [`Error::NotFound`] if no matching actor exists.
    pub fn resolve_actor_handle(&self, handle: &str) -> Result<ActorId> {
        let h = Handle::new(handle)?;
        self.find_actor_by_handle(&h)?
            .map(|a| a.id)
            .ok_or_else(|| Error::NotFound(handle.to_string()))
    }

    /// Resolve a commit signing key fingerprint to an actor UUID.
    ///
    /// # Errors
    /// Returns an error if a git operation fails.
    pub fn resolve_fingerprint(&self, fingerprint: &str) -> Result<Option<ActorId>> {
        for a in self.list_actors()? {
            if a.keys.iter().any(|k| k == fingerprint) {
                return Ok(Some(a.id));
            }
        }
        Ok(None)
    }

    /// Build a map from email address → actor UUID.
    ///
    /// # Errors
    /// Returns an error if a git operation fails.
    pub fn email_to_actor_map(&self) -> Result<HashMap<String, ActorId>> {
        let mut map = HashMap::new();
        for a in self.list_actors()? {
            for email in &a.emails {
                map.insert(email.clone(), a.id.clone());
            }
        }
        Ok(map)
    }

    /// List all actors.
    ///
    /// # Errors
    /// Returns an error if a git operation fails.
    pub fn list_actors(&self) -> Result<Vec<Actor>> {
        let mut out = Vec::new();
        if let Ok(refs) = self.repo.references_glob(&format!("{ACTORS_PREFIX}*")) {
            for r in refs.flatten() {
                let Some(name) = r.name() else { continue };
                let Some(suffix) = name.strip_prefix(ACTORS_PREFIX) else {
                    continue;
                };
                let Ok(id) = ActorId::parse(suffix) else {
                    continue;
                };
                if let Some(a) = read_actor(self.repo, &id)? {
                    out.push(a);
                }
            }
        }
        Ok(out)
    }

    /// Bootstrap: create the first actor from the repo's git identity,
    /// assigning the `admin` and `contributor` roles.
    ///
    /// # Errors
    /// Returns an error if `user.name` is not configured, actors already exist,
    /// or a git operation fails.
    pub fn bootstrap_actor(&self) -> Result<Actor> {
        if !self.list_actors()?.is_empty() {
            return Err(Error::Config("bootstrap: actors already exist".to_string()));
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
        self.create_actor(
            &handle,
            &[name.as_str()],
            &[email.as_str()],
            &["admin", "contributor"],
        )
    }

    /// Rename an actor's handle.
    ///
    /// # Errors
    /// Returns [`Error::NotFound`] if `old_handle` doesn't match any actor,
    /// or an error if `new_handle` is already taken.
    pub fn rename_actor(&self, old_handle: &str, new_handle: &str) -> Result<Actor> {
        let old = Handle::new(old_handle)?;
        let new = Handle::new(new_handle)?;

        let actor = self
            .find_actor_by_handle(&old)?
            .ok_or_else(|| Error::NotFound(old_handle.to_string()))?;

        if self.find_actor_by_handle(&new)?.is_some() {
            return Err(Error::Config(format!("handle already taken: {new}")));
        }

        let ref_name = actor_ref(&actor.id);
        let reference = self.repo.find_reference(&ref_name)?;
        let parent_commit = reference.peel_to_commit()?;
        let old_tree = parent_commit.tree()?;

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
            "rename actor",
            &new_tree,
            &[&parent_commit],
        )?;

        Ok(Actor {
            id: actor.id,
            handle: new,
            names: actor.names,
            emails: actor.emails,
            keys: actor.keys,
            attributes: actor.attributes,
            roles: actor.roles,
        })
    }

    /// Add a name to an actor.
    ///
    /// # Errors
    /// Returns [`Error::NotFound`] if the actor doesn't exist.
    pub fn add_actor_name(&self, handle: &str, name: &str) -> Result<Actor> {
        let h = Handle::new(handle)?;
        let actor = self
            .find_actor_by_handle(&h)?
            .ok_or_else(|| Error::NotFound(handle.to_string()))?;

        let ref_name = actor_ref(&actor.id);
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
            "add actor name",
            &new_tree,
            &[&parent_commit],
        )?;

        let mut names = actor.names;
        if !names.contains(&name.to_string()) {
            names.push(name.to_string());
        }
        Ok(Actor {
            id: actor.id,
            handle: h,
            names,
            emails: actor.emails,
            keys: actor.keys,
            attributes: actor.attributes,
            roles: actor.roles,
        })
    }

    /// Remove a name from an actor.
    ///
    /// # Errors
    /// Returns [`Error::NotFound`] if the actor or name doesn't exist.
    pub fn remove_actor_name(&self, handle: &str, name: &str) -> Result<Actor> {
        let h = Handle::new(handle)?;
        let actor = self
            .find_actor_by_handle(&h)?
            .ok_or_else(|| Error::NotFound(handle.to_string()))?;

        let ref_name = actor_ref(&actor.id);
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
            "remove actor name",
            &new_tree,
            &[&parent_commit],
        )?;

        let mut names = actor.names;
        names.retain(|n| n != name);
        Ok(Actor {
            id: actor.id,
            handle: h,
            names,
            emails: actor.emails,
            keys: actor.keys,
            attributes: actor.attributes,
            roles: actor.roles,
        })
    }

    /// Add an email to an actor.
    ///
    /// # Errors
    /// Returns [`Error::NotFound`] if the actor doesn't exist.
    pub fn add_actor_email(&self, handle: &str, email: &str) -> Result<Actor> {
        let h = Handle::new(handle)?;
        let actor = self
            .find_actor_by_handle(&h)?
            .ok_or_else(|| Error::NotFound(handle.to_string()))?;

        let ref_name = actor_ref(&actor.id);
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
            "add actor email",
            &new_tree,
            &[&parent_commit],
        )?;

        let mut emails = actor.emails;
        if !emails.contains(&email.to_string()) {
            emails.push(email.to_string());
        }
        Ok(Actor {
            id: actor.id,
            handle: h,
            names: actor.names,
            emails,
            keys: actor.keys,
            attributes: actor.attributes,
            roles: actor.roles,
        })
    }

    /// Remove an email from an actor.
    ///
    /// # Errors
    /// Returns [`Error::NotFound`] if the actor or email doesn't exist.
    pub fn remove_actor_email(&self, handle: &str, email: &str) -> Result<Actor> {
        let h = Handle::new(handle)?;
        let actor = self
            .find_actor_by_handle(&h)?
            .ok_or_else(|| Error::NotFound(handle.to_string()))?;

        let ref_name = actor_ref(&actor.id);
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
            "remove actor email",
            &new_tree,
            &[&parent_commit],
        )?;

        let mut emails = actor.emails;
        emails.retain(|e| e != email);
        Ok(Actor {
            id: actor.id,
            handle: h,
            names: actor.names,
            emails,
            keys: actor.keys,
            attributes: actor.attributes,
            roles: actor.roles,
        })
    }

    /// Add a public key to an actor.
    ///
    /// # Errors
    /// Returns [`Error::NotFound`] if the actor doesn't exist.
    pub fn add_actor_key(&self, handle: &str, fingerprint: &str, material: &[u8]) -> Result<Actor> {
        let h = Handle::new(handle)?;
        let actor = self
            .find_actor_by_handle(&h)?
            .ok_or_else(|| Error::NotFound(handle.to_string()))?;

        let ref_name = actor_ref(&actor.id);
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
            "add actor key",
            &new_tree,
            &[&parent_commit],
        )?;

        let mut keys = actor.keys;
        if !keys.contains(&fingerprint.to_string()) {
            keys.push(fingerprint.to_string());
        }
        Ok(Actor {
            id: actor.id,
            handle: h,
            names: actor.names,
            emails: actor.emails,
            keys,
            attributes: actor.attributes,
            roles: actor.roles,
        })
    }

    /// Remove a public key from an actor.
    ///
    /// # Errors
    /// Returns [`Error::NotFound`] if the actor or key doesn't exist.
    pub fn remove_actor_key(&self, handle: &str, fingerprint: &str) -> Result<Actor> {
        let h = Handle::new(handle)?;
        let actor = self
            .find_actor_by_handle(&h)?
            .ok_or_else(|| Error::NotFound(handle.to_string()))?;

        let ref_name = actor_ref(&actor.id);
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
            "remove actor key",
            &new_tree,
            &[&parent_commit],
        )?;

        let mut keys = actor.keys;
        keys.retain(|k| k != fingerprint);
        Ok(Actor {
            id: actor.id,
            handle: h,
            names: actor.names,
            emails: actor.emails,
            keys,
            attributes: actor.attributes,
            roles: actor.roles,
        })
    }

    /// Set an attribute on an actor (insert or update).
    ///
    /// # Errors
    /// Returns [`Error::NotFound`] if the actor doesn't exist.
    pub fn set_actor_attr(&self, handle: &str, key: &str, value: &str) -> Result<Actor> {
        validate_entry_name(key)?;
        let h = Handle::new(handle)?;
        let actor = self
            .find_actor_by_handle(&h)?
            .ok_or_else(|| Error::NotFound(handle.to_string()))?;

        let ref_name = actor_ref(&actor.id);
        let reference = self.repo.find_reference(&ref_name)?;
        let parent_commit = reference.peel_to_commit()?;
        let old_tree = parent_commit.tree()?;

        let blob_oid = self.repo.blob(value.as_bytes())?;
        let new_tree_oid = insert_into_subtree(self.repo, &old_tree, "attributes", key, blob_oid)?;
        let new_tree = self.repo.find_tree(new_tree_oid)?;

        let sig = self.repo.signature()?;
        self.repo.commit(
            Some(&ref_name),
            &sig,
            &sig,
            "set actor attr",
            &new_tree,
            &[&parent_commit],
        )?;

        let mut attributes = actor.attributes;
        attributes.insert(key.to_string(), value.to_string());
        Ok(Actor {
            id: actor.id,
            handle: h,
            names: actor.names,
            emails: actor.emails,
            keys: actor.keys,
            attributes,
            roles: actor.roles,
        })
    }

    /// Remove an attribute from an actor.
    ///
    /// # Errors
    /// Returns [`Error::NotFound`] if the actor or attribute doesn't exist.
    pub fn remove_actor_attr(&self, handle: &str, key: &str) -> Result<Actor> {
        let h = Handle::new(handle)?;
        let actor = self
            .find_actor_by_handle(&h)?
            .ok_or_else(|| Error::NotFound(handle.to_string()))?;

        let ref_name = actor_ref(&actor.id);
        let reference = self.repo.find_reference(&ref_name)?;
        let parent_commit = reference.peel_to_commit()?;
        let old_tree = parent_commit.tree()?;

        let new_tree_oid = drop_from_subtree(self.repo, &old_tree, "attributes", key)?;
        let new_tree = self.repo.find_tree(new_tree_oid)?;

        let sig = self.repo.signature()?;
        self.repo.commit(
            Some(&ref_name),
            &sig,
            &sig,
            "remove actor attr",
            &new_tree,
            &[&parent_commit],
        )?;

        let mut attributes = actor.attributes;
        attributes.remove(key);
        Ok(Actor {
            id: actor.id,
            handle: h,
            names: actor.names,
            emails: actor.emails,
            keys: actor.keys,
            attributes,
            roles: actor.roles,
        })
    }

    /// Add a role to an actor.
    ///
    /// # Errors
    /// Returns [`Error::NotFound`] if the actor doesn't exist.
    pub fn add_actor_role(&self, handle: &str, role: &str) -> Result<Actor> {
        let h = Handle::new(handle)?;
        let actor = self
            .find_actor_by_handle(&h)?
            .ok_or_else(|| Error::NotFound(handle.to_string()))?;

        let ref_name = actor_ref(&actor.id);
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
            "add actor role",
            &new_tree,
            &[&parent_commit],
        )?;

        let mut roles = actor.roles;
        if !roles.contains(&role.to_string()) {
            roles.push(role.to_string());
        }
        Ok(Actor {
            id: actor.id,
            handle: h,
            names: actor.names,
            emails: actor.emails,
            keys: actor.keys,
            attributes: actor.attributes,
            roles,
        })
    }

    /// Remove a role from an actor.
    ///
    /// # Errors
    /// Returns [`Error::NotFound`] if the actor or role doesn't exist.
    pub fn remove_actor_role(&self, handle: &str, role: &str) -> Result<Actor> {
        let h = Handle::new(handle)?;
        let actor = self
            .find_actor_by_handle(&h)?
            .ok_or_else(|| Error::NotFound(handle.to_string()))?;

        let ref_name = actor_ref(&actor.id);
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
            "remove actor role",
            &new_tree,
            &[&parent_commit],
        )?;

        let mut roles = actor.roles;
        roles.retain(|r| r != role);
        Ok(Actor {
            id: actor.id,
            handle: h,
            names: actor.names,
            emails: actor.emails,
            keys: actor.keys,
            attributes: actor.attributes,
            roles,
        })
    }
}
