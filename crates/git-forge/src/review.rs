//! Review entity CRUD backed by `git-ledger`.

use facet::Facet;
use git_ledger::{IdStrategy, Ledger, LedgerEntry, Mutation};
use git2::{FileMode, ObjectType, Repository};
use serde::Serialize;

use crate::index::{display_id_for_oid, index_upsert, read_index, resolve_oid};
use crate::refs::{REVIEW_INDEX, REVIEW_PREFIX};
use crate::{Error, Result, Store};

/// The lifecycle state of a review.
#[derive(Debug, Clone, Serialize, Facet, PartialEq, Eq)]
#[repr(u8)]
#[serde(rename_all = "lowercase")]
pub enum ReviewState {
    /// Review is open and accepting comments.
    Open,
    /// Review has been closed.
    Closed,
}

impl ReviewState {
    /// Return the canonical string representation.
    #[must_use]
    pub fn as_str(&self) -> &'static str {
        match self {
            ReviewState::Open => "open",
            ReviewState::Closed => "closed",
        }
    }
}

impl std::str::FromStr for ReviewState {
    type Err = Error;

    fn from_str(s: &str) -> Result<Self> {
        match s {
            "open" => Ok(ReviewState::Open),
            "closed" => Ok(ReviewState::Closed),
            _ => Err(Error::InvalidState(s.to_string())),
        }
    }
}

/// The target objects for a review.
#[derive(Debug, Clone, Serialize, Facet)]
pub struct ReviewTarget {
    /// The head object OID (required).
    pub head: String,
    /// The base object OID (optional — absent for single-object reviews).
    pub base: Option<String>,
}

/// A forge review backed by a git-ledger entity ref.
#[derive(Debug, Clone, Serialize, Facet)]
pub struct Review {
    /// Permanent identity: the OID of the initial commit on the entity ref.
    pub oid: String,
    /// Display ID (`"GH#1"` for GitHub-synced). `None` while pending sync.
    pub display_id: Option<String>,
    /// Review title.
    pub title: String,
    /// Target objects.
    pub target: ReviewTarget,
    /// Optional source ref name (e.g. `"feature-branch"`).
    pub source_ref: Option<String>,
    /// Current state.
    pub state: ReviewState,
    /// Description in Markdown.
    pub description: String,
    /// Approvals keyed by contributor name, with an optional message.
    pub approvals: Vec<(String, String)>,
}

fn review_from_entry(entry: &LedgerEntry, display_id: Option<String>) -> Result<Review> {
    let mut title = String::new();
    let mut state = ReviewState::Open;
    let mut description = String::new();
    let mut source_ref: Option<String> = None;
    let mut head = String::new();
    let mut base: Option<String> = None;
    let mut approvals: Vec<(String, String)> = Vec::new();

    for (name, value) in &entry.fields {
        let text = || String::from_utf8_lossy(value).into_owned();
        match name.as_str() {
            "title" => title = text(),
            "description" => description = text(),
            "meta/state" => state = text().parse()?,
            "meta/ref" => source_ref = Some(text()),
            "meta/target/head" => head = text(),
            "meta/target/base" => base = Some(text()),
            _ if name.starts_with("approvals/") => {
                let user = name.strip_prefix("approvals/").unwrap_or("");
                if !user.is_empty() {
                    approvals.push((user.to_string(), text()));
                }
            }
            _ => {}
        }
    }

    Ok(Review {
        oid: entry.id.clone(),
        display_id,
        title,
        target: ReviewTarget { head, base },
        source_ref,
        state,
        description,
        approvals,
    })
}

/// Placeholder fields for the `objects/` subtree.
///
/// These reserve the tree paths during `Ledger::create`; `fixup_pin_entries`
/// replaces them with tree entries whose OID is the actual target object.
fn pin_fields(repo: &Repository, target: &ReviewTarget) -> Result<Vec<(String, Vec<u8>)>> {
    let mut fields = Vec::new();
    let head_oid = git2::Oid::from_str(&target.head)?;
    repo.find_object(head_oid, None)?; // validate existence
    fields.push((format!("objects/{}", target.head), Vec::new()));
    if let Some(ref base) = target.base {
        let base_oid = git2::Oid::from_str(base)?;
        repo.find_object(base_oid, None)?;
        fields.push((format!("objects/{base}"), Vec::new()));
    }
    Ok(fields)
}

/// Like `pin_fields` but silently skips objects that are not in the local repo.
fn try_pin_fields(repo: &Repository, target: &ReviewTarget) -> Vec<(String, Vec<u8>)> {
    let mut fields = Vec::new();
    if let Ok(oid) = git2::Oid::from_str(&target.head)
        && repo.find_object(oid, None).is_ok()
    {
        fields.push((format!("objects/{}", target.head), Vec::new()));
    }
    if let Some(ref base) = target.base
        && let Ok(oid) = git2::Oid::from_str(base)
        && repo.find_object(oid, None).is_ok()
    {
        fields.push((format!("objects/{base}"), Vec::new()));
    }
    fields
}

/// Map a git object type to the tree entry file mode used in the `objects/` tree.
fn object_file_mode(kind: Option<ObjectType>) -> FileMode {
    match kind {
        Some(ObjectType::Tree) => FileMode::Tree,
        Some(ObjectType::Commit) => FileMode::Commit,
        _ => FileMode::Blob,
    }
}

/// Rewrite the `objects/` subtree so each entry's OID points to the actual
/// git object rather than the placeholder blob created by `Ledger::create`.
///
/// This is necessary because `git-ledger` always creates new blobs for field
/// values; it cannot insert tree entries pointing to arbitrary existing objects.
fn fixup_pin_entries(repo: &Repository, review_oid: &str, target: &ReviewTarget) -> Result<()> {
    let ref_name = format!("{REVIEW_PREFIX}{review_oid}");
    let reference = repo.find_reference(&ref_name)?;
    let commit = reference.peel_to_commit()?;
    let root_tree = commit.tree()?;

    // Collect OIDs to pin (only those present locally).
    let mut pins: Vec<(&str, git2::Oid, FileMode)> = Vec::new();
    if let Ok(oid) = git2::Oid::from_str(&target.head)
        && let Ok(obj) = repo.find_object(oid, None)
    {
        pins.push((&target.head, oid, object_file_mode(obj.kind())));
    }
    if let Some(ref base) = target.base
        && let Ok(oid) = git2::Oid::from_str(base)
        && let Ok(obj) = repo.find_object(oid, None)
    {
        pins.push((base, oid, object_file_mode(obj.kind())));
    }

    if pins.is_empty() {
        return Ok(());
    }

    // Build the objects/ subtree with correct OIDs and modes.
    // Blobs get mode 100644, trees get 040000, commits get 160000 (gitlink).
    // All three modes create a direct reference to the object, preventing GC.
    let existing_objects = root_tree.get_name("objects").and_then(|e| {
        if e.kind() == Some(ObjectType::Tree) {
            repo.find_tree(e.id()).ok()
        } else {
            None
        }
    });
    let mut obj_builder = repo.treebuilder(existing_objects.as_ref())?;
    for (name, oid, mode) in &pins {
        obj_builder.insert(name, *oid, i32::from(*mode))?;
    }
    let obj_tree_oid = obj_builder.write()?;

    // Rebuild root tree with the fixed objects/ entry.
    let mut root_builder = repo.treebuilder(Some(&root_tree))?;
    root_builder.insert("objects", obj_tree_oid, 0o040_000)?;
    let new_tree_oid = root_builder.write()?;
    let new_tree = repo.find_tree(new_tree_oid)?;

    // Amend: create new commit with same parent(s), author, message.
    let sig = commit.author();
    let parents: Vec<git2::Commit<'_>> = commit
        .parent_ids()
        .filter_map(|id| repo.find_commit(id).ok())
        .collect();
    let parent_refs: Vec<&git2::Commit<'_>> = parents.iter().collect();
    let new_commit = repo.commit(
        None,
        &sig,
        &sig,
        commit.message().unwrap_or(""),
        &new_tree,
        &parent_refs,
    )?;

    // Update the ref to point to the amended commit.
    repo.reference(&ref_name, new_commit, true, "fixup pin entries")?;
    Ok(())
}

impl Store<'_> {
    /// Create a new review, writing an OID-keyed entity ref and staging it in the index.
    ///
    /// # Errors
    /// Returns an error if a git operation fails.
    pub fn create_review(
        &self,
        title: &str,
        description: &str,
        target: &ReviewTarget,
        source_ref: Option<&str>,
    ) -> Result<Review> {
        let mut fields: Vec<(&str, &[u8])> = vec![
            ("title", title.as_bytes()),
            ("description", description.as_bytes()),
            ("meta/state", b"open"),
            ("meta/target/head", target.head.as_bytes()),
        ];
        if let Some(ref base) = target.base {
            fields.push(("meta/target/base", base.as_bytes()));
        }
        if let Some(sref) = source_ref {
            fields.push(("meta/ref", sref.as_bytes()));
        }

        let pin = pin_fields(self.repo, target)?;
        let pin_refs: Vec<(&str, &[u8])> = pin
            .iter()
            .map(|(k, v)| (k.as_str(), v.as_slice()))
            .collect();
        fields.extend(pin_refs);

        let entry = self.repo.create(
            REVIEW_PREFIX,
            &IdStrategy::CommitOid,
            &fields,
            "create review",
            None,
        )?;

        fixup_pin_entries(self.repo, &entry.id, target)?;

        Ok(Review {
            oid: entry.id,
            display_id: None,
            title: title.to_string(),
            target: target.clone(),
            source_ref: source_ref.map(String::from),
            state: ReviewState::Open,
            description: description.to_string(),
            approvals: Vec::new(),
        })
    }

    /// Create a review from an external source with a custom author.
    ///
    /// `display_id` is written to the index immediately. Attempts to pin target
    /// objects; silently skips pinning for objects not available locally.
    ///
    /// # Errors
    /// Returns an error if a git operation fails.
    #[allow(clippy::too_many_arguments)]
    pub fn create_review_imported(
        &self,
        title: &str,
        description: &str,
        target: &ReviewTarget,
        source_ref: Option<&str>,
        state: Option<&ReviewState>,
        display_id: &str,
        author: &git2::Signature<'_>,
        source: &str,
    ) -> Result<Review> {
        let state = state.cloned().unwrap_or(ReviewState::Open);
        let state_str = state.as_str().to_string();

        let mut fields: Vec<(&str, &[u8])> = vec![
            ("title", title.as_bytes()),
            ("description", description.as_bytes()),
            ("meta/state", state_str.as_bytes()),
            ("meta/target/head", target.head.as_bytes()),
            ("source/url", source.as_bytes()),
        ];
        if let Some(ref base) = target.base {
            fields.push(("meta/target/base", base.as_bytes()));
        }
        if let Some(sref) = source_ref {
            fields.push(("meta/ref", sref.as_bytes()));
        }

        let pin = try_pin_fields(self.repo, target);
        let pin_refs: Vec<(&str, &[u8])> = pin
            .iter()
            .map(|(k, v)| (k.as_str(), v.as_slice()))
            .collect();
        fields.extend(pin_refs);

        let entry = self.repo.create(
            REVIEW_PREFIX,
            &IdStrategy::CommitOid,
            &fields,
            "forge: create review",
            Some(author),
        )?;

        let oid = entry.id.clone();
        index_upsert(self.repo, REVIEW_INDEX, &[(display_id, &oid)])?;
        fixup_pin_entries(self.repo, &oid, target)?;

        Ok(Review {
            oid,
            display_id: Some(display_id.to_string()),
            title: title.to_string(),
            target: target.clone(),
            source_ref: source_ref.map(String::from),
            state,
            description: description.to_string(),
            approvals: Vec::new(),
        })
    }

    /// Fetch a single review by display ID or OID prefix.
    ///
    /// # Errors
    /// Returns [`Error::NotFound`] if the review does not exist, or a git error on failure.
    pub fn get_review(&self, oid_or_id: &str) -> Result<Review> {
        let index = read_index(self.repo, REVIEW_INDEX)?;
        let known_oids = self.repo.list(REVIEW_PREFIX)?;
        let oid = resolve_oid(index.as_ref(), &known_oids, oid_or_id)?;
        let ref_name = format!("{REVIEW_PREFIX}{oid}");
        let entry = self.repo.read(&ref_name)?;
        let display_id = display_id_for_oid(index.as_ref(), &oid);
        review_from_entry(&entry, display_id)
    }

    /// List all reviews in the repository.
    ///
    /// # Errors
    /// Returns an error if any git operation fails.
    pub fn list_reviews(&self) -> Result<Vec<Review>> {
        let index = read_index(self.repo, REVIEW_INDEX)?;
        let oids = self.repo.list(REVIEW_PREFIX)?;
        oids.into_iter()
            .map(|oid| {
                let ref_name = format!("{REVIEW_PREFIX}{oid}");
                let entry = self.repo.read(&ref_name)?;
                let display_id = display_id_for_oid(index.as_ref(), &oid);
                review_from_entry(&entry, display_id)
            })
            .collect()
    }

    /// List reviews filtered by state.
    ///
    /// # Errors
    /// Returns an error if any git operation fails.
    pub fn list_reviews_by_state(&self, state: &ReviewState) -> Result<Vec<Review>> {
        Ok(self
            .list_reviews()?
            .into_iter()
            .filter(|r| &r.state == state)
            .collect())
    }

    /// Apply a partial update to a review.
    ///
    /// # Errors
    /// Returns [`Error::NotFound`] if the review does not exist, or a git error on failure.
    pub fn update_review(
        &self,
        oid_or_id: &str,
        title: Option<&str>,
        description: Option<&str>,
        state: Option<&ReviewState>,
    ) -> Result<Review> {
        let index = read_index(self.repo, REVIEW_INDEX)?;
        let known_oids = self.repo.list(REVIEW_PREFIX)?;
        let oid = resolve_oid(index.as_ref(), &known_oids, oid_or_id)?;
        let ref_name = format!("{REVIEW_PREFIX}{oid}");

        let state_bytes: Option<String> = state.map(|s| s.as_str().to_string());

        let mut mutations: Vec<Mutation<'_>> = Vec::new();
        if let Some(t) = title {
            mutations.push(Mutation::Set("title", t.as_bytes()));
        }
        if let Some(d) = description {
            mutations.push(Mutation::Set("description", d.as_bytes()));
        }
        if let Some(ref s) = state_bytes {
            mutations.push(Mutation::Set("meta/state", s.as_bytes()));
        }

        let entry = self.repo.update(&ref_name, &mutations, "update review")?;
        let display_id = display_id_for_oid(index.as_ref(), &oid);
        review_from_entry(&entry, display_id)
    }

    /// Re-resolve `meta/ref` to update `meta/target/*` and `objects/`.
    ///
    /// No-op if the review has no `meta/ref`.
    ///
    /// # Errors
    /// Returns an error if the review does not exist or a git operation fails.
    pub fn refresh_review_target(&self, oid_or_id: &str) -> Result<Review> {
        let index = read_index(self.repo, REVIEW_INDEX)?;
        let known_oids = self.repo.list(REVIEW_PREFIX)?;
        let oid = resolve_oid(index.as_ref(), &known_oids, oid_or_id)?;
        let ref_name = format!("{REVIEW_PREFIX}{oid}");
        let entry = self.repo.read(&ref_name)?;
        let display_id = display_id_for_oid(index.as_ref(), &oid);
        let review = review_from_entry(&entry, display_id.clone())?;

        let Some(ref sref) = review.source_ref else {
            return Ok(review);
        };

        let git_ref = self
            .repo
            .find_reference(sref)
            .or_else(|_| self.repo.find_reference(&format!("refs/heads/{sref}")))?;
        let new_head = git_ref.peel_to_commit()?.id().to_string();

        let new_target = ReviewTarget {
            head: new_head.clone(),
            base: review.target.base.clone(),
        };
        let pin = pin_fields(self.repo, &new_target)?;

        let mut mutations: Vec<Mutation<'_>> =
            vec![Mutation::Set("meta/target/head", new_head.as_bytes())];
        let pin_refs: Vec<(&str, &[u8])> = pin
            .iter()
            .map(|(k, v)| (k.as_str(), v.as_slice()))
            .collect();
        for (k, v) in &pin_refs {
            mutations.push(Mutation::Set(k, v));
        }

        let entry = self
            .repo
            .update(&ref_name, &mutations, "refresh review target")?;
        fixup_pin_entries(self.repo, &oid, &new_target)?;
        review_from_entry(&entry, display_id)
    }

    /// Update a review's target head to a new OID.
    ///
    /// Returns the old head OID and the updated review so the caller can diff
    /// trees and migrate carry-forward comments.
    ///
    /// # Errors
    /// Returns an error if the review does not exist or a git operation fails.
    pub fn retarget_review(&self, oid_or_id: &str, new_head: &str) -> Result<(String, Review)> {
        let index = read_index(self.repo, REVIEW_INDEX)?;
        let known_oids = self.repo.list(REVIEW_PREFIX)?;
        let oid = resolve_oid(index.as_ref(), &known_oids, oid_or_id)?;
        let ref_name = format!("{REVIEW_PREFIX}{oid}");
        let entry = self.repo.read(&ref_name)?;
        let display_id = display_id_for_oid(index.as_ref(), &oid);
        let old_review = review_from_entry(&entry, display_id.clone())?;
        let old_head = old_review.target.head.clone();

        let new_target = ReviewTarget {
            head: new_head.to_string(),
            base: old_review.target.base.clone(),
        };
        let pin = pin_fields(self.repo, &new_target)?;

        let mut mutations: Vec<Mutation<'_>> =
            vec![Mutation::Set("meta/target/head", new_head.as_bytes())];
        let pin_refs: Vec<(&str, &[u8])> = pin
            .iter()
            .map(|(k, v)| (k.as_str(), v.as_slice()))
            .collect();
        for (k, v) in &pin_refs {
            mutations.push(Mutation::Set(k, v));
        }

        let entry = self.repo.update(&ref_name, &mutations, "retarget review")?;
        fixup_pin_entries(self.repo, &oid, &new_target)?;
        let review = review_from_entry(&entry, display_id)?;
        Ok((old_head, review))
    }

    /// Record an approval on a review from the current git user.
    ///
    /// The approval is stored as `approvals/<name>` in the review's ledger
    /// entry. Approving again overwrites the previous approval message.
    ///
    /// # Errors
    /// Returns an error if the review does not exist or a git operation fails.
    pub fn approve_review(&self, oid_or_id: &str, message: Option<&str>) -> Result<Review> {
        let index = read_index(self.repo, REVIEW_INDEX)?;
        let known_oids = self.repo.list(REVIEW_PREFIX)?;
        let oid = resolve_oid(index.as_ref(), &known_oids, oid_or_id)?;
        let ref_name = format!("{REVIEW_PREFIX}{oid}");

        let cfg = self.repo.config()?;
        let name = cfg
            .get_string("user.name")
            .map_err(|_| Error::Config("user.name not set".into()))?;

        let field = format!("approvals/{name}");
        let body = message.unwrap_or("").as_bytes();
        let mutations = [Mutation::Set(&field, body)];
        let entry = self.repo.update(&ref_name, &mutations, "approve review")?;
        let display_id = display_id_for_oid(index.as_ref(), &oid);
        review_from_entry(&entry, display_id)
    }

    /// Revoke an approval on a review from the current git user.
    ///
    /// # Errors
    /// Returns an error if the review does not exist or a git operation fails.
    pub fn revoke_approval(&self, oid_or_id: &str) -> Result<Review> {
        let index = read_index(self.repo, REVIEW_INDEX)?;
        let known_oids = self.repo.list(REVIEW_PREFIX)?;
        let oid = resolve_oid(index.as_ref(), &known_oids, oid_or_id)?;
        let ref_name = format!("{REVIEW_PREFIX}{oid}");

        let cfg = self.repo.config()?;
        let name = cfg
            .get_string("user.name")
            .map_err(|_| Error::Config("user.name not set".into()))?;

        let field = format!("approvals/{name}");
        let mutations = [Mutation::Delete(&field)];
        let entry = self.repo.update(&ref_name, &mutations, "revoke approval")?;
        let display_id = display_id_for_oid(index.as_ref(), &oid);
        review_from_entry(&entry, display_id)
    }
}
