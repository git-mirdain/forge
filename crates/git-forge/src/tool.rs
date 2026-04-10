//! Tool model backed by `refs/forge/tools/<uuid-v7>`.
//!
//! Each tool ref points to a commit whose tree has the layout:
//!
//! ```text
//! ├── handle          # blob: mutable display name, must be unique
//! ├── name            # blob: canonical git identity name
//! ├── email           # blob: canonical git identity email
//! ├── aliases/
//! │   └── <name>      # empty blob — alternate names in git author/committer field
//! ├── attributes/
//! │   └── <key>       # blob: attribute value (e.g. vendor, model, type)
//! └── roles/
//!     └── <role>      # empty blob — presence means role is granted
//! ```
//!
//! Timestamps live in the commit metadata.

use std::collections::BTreeMap;

use facet::Facet;
use git2::{ErrorCode, ObjectType, Repository};
use uuid::Uuid;

use crate::contributor::{
    Handle, drop_from_subtree, insert_into_subtree, read_subtree_keys, validate_entry_name,
};
use crate::refs::TOOLS_PREFIX;
use crate::{Error, Result, Store};

// ── newtype ──────────────────────────────────────────────────────────────────

/// Opaque UUID v7 identifier for a tool.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Facet)]
pub struct ToolId(String);

impl ToolId {
    /// Generate a fresh UUID v7 tool ID.
    #[must_use]
    pub fn new() -> Self {
        Self(Uuid::now_v7().to_string())
    }

    /// Wrap an existing UUID string without re-generating.
    ///
    /// # Errors
    /// Returns an error if `s` is not a valid UUID.
    pub fn parse(s: &str) -> Result<Self> {
        Uuid::parse_str(s).map_err(|_| Error::Config(format!("invalid tool id: {s}")))?;
        Ok(Self(s.to_string()))
    }

    /// Return the inner UUID string.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl Default for ToolId {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Display for ToolId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

// ── entity ───────────────────────────────────────────────────────────────────

/// A non-human tool stored at `refs/forge/tools/<uuid-v7>`.
#[derive(Debug, Clone, Facet)]
pub struct Tool {
    /// Stable UUID v7 identity.
    pub id: ToolId,
    /// Mutable display handle.
    pub handle: Handle,
    /// Canonical git identity name (appears in Author/Committer fields).
    pub name: String,
    /// Canonical git identity email.
    pub email: String,
    /// Alternate names matched in git author/committer fields.
    pub aliases: Vec<String>,
    /// Arbitrary key→value attributes (e.g. `vendor`, `model`, `type`).
    pub attributes: BTreeMap<String, String>,
    /// Roles present in `roles/`.
    pub roles: Vec<String>,
}

// ── internal helpers ─────────────────────────────────────────────────────────

fn tool_ref(id: &ToolId) -> String {
    format!("{TOOLS_PREFIX}{}", id.as_str())
}

/// Read key→value pairs from a subtree whose blob content is the value.
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

fn read_tool(repo: &Repository, id: &ToolId) -> Result<Option<Tool>> {
    let ref_name = tool_ref(id);
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

    let name = tree
        .get_name("name")
        .and_then(|e| repo.find_blob(e.id()).ok())
        .map(|b| String::from_utf8_lossy(b.content()).into_owned())
        .unwrap_or_default();

    let email = tree
        .get_name("email")
        .and_then(|e| repo.find_blob(e.id()).ok())
        .map(|b| String::from_utf8_lossy(b.content()).into_owned())
        .unwrap_or_default();

    let aliases = read_subtree_keys(repo, &tree, "aliases");
    let attributes = read_subtree_kv(repo, &tree, "attributes");
    let roles = read_subtree_keys(repo, &tree, "roles");

    Ok(Some(Tool {
        id: id.clone(),
        handle,
        name,
        email,
        aliases,
        attributes,
        roles,
    }))
}

struct NewTool<'a> {
    handle: &'a Handle,
    name: &'a str,
    email: &'a str,
    aliases: &'a [&'a str],
    attributes: &'a [(&'a str, &'a str)],
    roles: &'a [&'a str],
}

fn write_tool(repo: &Repository, id: &ToolId, fields: &NewTool<'_>, message: &str) -> Result<()> {
    let empty = repo.blob(b"")?;
    let mut builder = repo.treebuilder(None)?;

    let handle_oid = repo.blob(fields.handle.as_str().as_bytes())?;
    builder.insert("handle", handle_oid, 0o100_644)?;

    let name_oid = repo.blob(fields.name.as_bytes())?;
    builder.insert("name", name_oid, 0o100_644)?;

    let email_oid = repo.blob(fields.email.as_bytes())?;
    builder.insert("email", email_oid, 0o100_644)?;

    if !fields.aliases.is_empty() {
        let mut ab = repo.treebuilder(None)?;
        for alias in fields.aliases {
            ab.insert(alias, empty, 0o100_644)?;
        }
        builder.insert("aliases", ab.write()?, 0o040_000)?;
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
    let ref_name = tool_ref(id);
    repo.commit(Some(&ref_name), &sig, &sig, message, &tree, &[])?;
    Ok(())
}

// ── Store impl ────────────────────────────────────────────────────────────────

impl Store<'_> {
    /// Create a new tool.
    ///
    /// # Errors
    /// Returns an error if the handle is invalid, already taken, or a git
    /// operation fails.
    pub fn create_tool(
        &self,
        handle: &str,
        name: &str,
        email: &str,
        aliases: &[&str],
        attributes: &[(&str, &str)],
        roles: &[&str],
    ) -> Result<Tool> {
        let handle = Handle::new(handle)?;
        if self.find_tool_by_handle(&handle)?.is_some() {
            return Err(Error::Config(format!("handle already taken: {handle}")));
        }
        let id = ToolId::new();
        write_tool(
            self.repo,
            &id,
            &NewTool {
                handle: &handle,
                name,
                email,
                aliases,
                attributes,
                roles,
            },
            "create tool",
        )?;
        Ok(Tool {
            id,
            handle,
            name: name.to_string(),
            email: email.to_string(),
            aliases: aliases.iter().map(ToString::to_string).collect(),
            attributes: attributes
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect(),
            roles: roles.iter().map(ToString::to_string).collect(),
        })
    }

    /// Fetch a tool by UUID string.
    ///
    /// # Errors
    /// Returns [`Error::NotFound`] if no matching tool exists.
    pub fn get_tool(&self, id_str: &str) -> Result<Tool> {
        let id = ToolId::parse(id_str)?;
        read_tool(self.repo, &id)?.ok_or_else(|| Error::NotFound(id_str.to_string()))
    }

    /// Resolve a handle to a tool.
    ///
    /// # Errors
    /// Returns an error if a git operation fails.
    pub fn find_tool_by_handle(&self, handle: &Handle) -> Result<Option<Tool>> {
        for t in self.list_tools()? {
            if t.handle == *handle {
                return Ok(Some(t));
            }
        }
        Ok(None)
    }

    /// Resolve a handle string to a tool UUID.
    ///
    /// # Errors
    /// Returns [`Error::NotFound`] if no matching tool exists.
    pub fn resolve_tool_handle(&self, handle: &str) -> Result<ToolId> {
        let h = Handle::new(handle)?;
        self.find_tool_by_handle(&h)?
            .map(|t| t.id)
            .ok_or_else(|| Error::NotFound(handle.to_string()))
    }

    /// List all tools.
    ///
    /// # Errors
    /// Returns an error if a git operation fails.
    pub fn list_tools(&self) -> Result<Vec<Tool>> {
        let mut out = Vec::new();
        if let Ok(refs) = self.repo.references_glob(&format!("{TOOLS_PREFIX}*")) {
            for r in refs.flatten() {
                let Some(name) = r.name() else { continue };
                let Some(suffix) = name.strip_prefix(TOOLS_PREFIX) else {
                    continue;
                };
                let Ok(id) = ToolId::parse(suffix) else {
                    continue;
                };
                if let Some(t) = read_tool(self.repo, &id)? {
                    out.push(t);
                }
            }
        }
        Ok(out)
    }

    /// Rename a tool's handle.
    ///
    /// # Errors
    /// Returns [`Error::NotFound`] if `old_handle` doesn't match any tool,
    /// or an error if `new_handle` is already taken.
    pub fn rename_tool(&self, old_handle: &str, new_handle: &str) -> Result<Tool> {
        let old = Handle::new(old_handle)?;
        let new = Handle::new(new_handle)?;

        let tool = self
            .find_tool_by_handle(&old)?
            .ok_or_else(|| Error::NotFound(old_handle.to_string()))?;

        if self.find_tool_by_handle(&new)?.is_some() {
            return Err(Error::Config(format!("handle already taken: {new}")));
        }

        let ref_name = tool_ref(&tool.id);
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
            "rename tool",
            &new_tree,
            &[&parent_commit],
        )?;

        Ok(Tool {
            id: tool.id,
            handle: new,
            name: tool.name,
            email: tool.email,
            aliases: tool.aliases,
            attributes: tool.attributes,
            roles: tool.roles,
        })
    }

    /// Add an alias to a tool.
    ///
    /// # Errors
    /// Returns [`Error::NotFound`] if the tool doesn't exist.
    pub fn add_tool_alias(&self, handle: &str, alias: &str) -> Result<Tool> {
        validate_entry_name(alias)?;
        let h = Handle::new(handle)?;
        let tool = self
            .find_tool_by_handle(&h)?
            .ok_or_else(|| Error::NotFound(handle.to_string()))?;

        let ref_name = tool_ref(&tool.id);
        let reference = self.repo.find_reference(&ref_name)?;
        let parent_commit = reference.peel_to_commit()?;
        let old_tree = parent_commit.tree()?;

        let empty_oid = self.repo.blob(b"")?;
        let new_tree_oid = insert_into_subtree(self.repo, &old_tree, "aliases", alias, empty_oid)?;
        let new_tree = self.repo.find_tree(new_tree_oid)?;

        let sig = self.repo.signature()?;
        self.repo.commit(
            Some(&ref_name),
            &sig,
            &sig,
            "add tool alias",
            &new_tree,
            &[&parent_commit],
        )?;

        let mut aliases = tool.aliases;
        if !aliases.contains(&alias.to_string()) {
            aliases.push(alias.to_string());
        }
        Ok(Tool {
            id: tool.id,
            handle: h,
            name: tool.name,
            email: tool.email,
            aliases,
            attributes: tool.attributes,
            roles: tool.roles,
        })
    }

    /// Remove an alias from a tool.
    ///
    /// # Errors
    /// Returns [`Error::NotFound`] if the tool or alias doesn't exist.
    pub fn remove_tool_alias(&self, handle: &str, alias: &str) -> Result<Tool> {
        let h = Handle::new(handle)?;
        let tool = self
            .find_tool_by_handle(&h)?
            .ok_or_else(|| Error::NotFound(handle.to_string()))?;

        let ref_name = tool_ref(&tool.id);
        let reference = self.repo.find_reference(&ref_name)?;
        let parent_commit = reference.peel_to_commit()?;
        let old_tree = parent_commit.tree()?;

        let new_tree_oid = drop_from_subtree(self.repo, &old_tree, "aliases", alias)?;
        let new_tree = self.repo.find_tree(new_tree_oid)?;

        let sig = self.repo.signature()?;
        self.repo.commit(
            Some(&ref_name),
            &sig,
            &sig,
            "remove tool alias",
            &new_tree,
            &[&parent_commit],
        )?;

        let mut aliases = tool.aliases;
        aliases.retain(|a| a != alias);
        Ok(Tool {
            id: tool.id,
            handle: h,
            name: tool.name,
            email: tool.email,
            aliases,
            attributes: tool.attributes,
            roles: tool.roles,
        })
    }

    /// Set an attribute on a tool (insert or update).
    ///
    /// # Errors
    /// Returns [`Error::NotFound`] if the tool doesn't exist.
    pub fn set_tool_attr(&self, handle: &str, key: &str, value: &str) -> Result<Tool> {
        validate_entry_name(key)?;
        let h = Handle::new(handle)?;
        let tool = self
            .find_tool_by_handle(&h)?
            .ok_or_else(|| Error::NotFound(handle.to_string()))?;

        let ref_name = tool_ref(&tool.id);
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
            "set tool attr",
            &new_tree,
            &[&parent_commit],
        )?;

        let mut attributes = tool.attributes;
        attributes.insert(key.to_string(), value.to_string());
        Ok(Tool {
            id: tool.id,
            handle: h,
            name: tool.name,
            email: tool.email,
            aliases: tool.aliases,
            attributes,
            roles: tool.roles,
        })
    }

    /// Remove an attribute from a tool.
    ///
    /// # Errors
    /// Returns [`Error::NotFound`] if the tool or attribute doesn't exist.
    pub fn remove_tool_attr(&self, handle: &str, key: &str) -> Result<Tool> {
        let h = Handle::new(handle)?;
        let tool = self
            .find_tool_by_handle(&h)?
            .ok_or_else(|| Error::NotFound(handle.to_string()))?;

        let ref_name = tool_ref(&tool.id);
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
            "remove tool attr",
            &new_tree,
            &[&parent_commit],
        )?;

        let mut attributes = tool.attributes;
        attributes.remove(key);
        Ok(Tool {
            id: tool.id,
            handle: h,
            name: tool.name,
            email: tool.email,
            aliases: tool.aliases,
            attributes,
            roles: tool.roles,
        })
    }

    /// Add a role to a tool.
    ///
    /// # Errors
    /// Returns [`Error::NotFound`] if the tool doesn't exist.
    pub fn add_tool_role(&self, handle: &str, role: &str) -> Result<Tool> {
        let h = Handle::new(handle)?;
        let tool = self
            .find_tool_by_handle(&h)?
            .ok_or_else(|| Error::NotFound(handle.to_string()))?;

        let ref_name = tool_ref(&tool.id);
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
            "add tool role",
            &new_tree,
            &[&parent_commit],
        )?;

        let mut roles = tool.roles;
        if !roles.contains(&role.to_string()) {
            roles.push(role.to_string());
        }
        Ok(Tool {
            id: tool.id,
            handle: h,
            name: tool.name,
            email: tool.email,
            aliases: tool.aliases,
            attributes: tool.attributes,
            roles,
        })
    }

    /// Remove a role from a tool.
    ///
    /// # Errors
    /// Returns [`Error::NotFound`] if the tool or role doesn't exist.
    pub fn remove_tool_role(&self, handle: &str, role: &str) -> Result<Tool> {
        let h = Handle::new(handle)?;
        let tool = self
            .find_tool_by_handle(&h)?
            .ok_or_else(|| Error::NotFound(handle.to_string()))?;

        let ref_name = tool_ref(&tool.id);
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
            "remove tool role",
            &new_tree,
            &[&parent_commit],
        )?;

        let mut roles = tool.roles;
        roles.retain(|r| r != role);
        Ok(Tool {
            id: tool.id,
            handle: h,
            name: tool.name,
            email: tool.email,
            aliases: tool.aliases,
            attributes: tool.attributes,
            roles,
        })
    }
}
