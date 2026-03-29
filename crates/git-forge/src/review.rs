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
    /// Review is open and active.
    Open,
    /// Review has been merged.
    Merged,
    /// Review has been closed without merging.
    Closed,
}

impl ReviewState {
    /// Return the canonical string representation.
    #[must_use]
    pub fn as_str(&self) -> &'static str {
        match self {
            ReviewState::Open => "open",
            ReviewState::Merged => "merged",
            ReviewState::Closed => "closed",
        }
    }
}

impl std::str::FromStr for ReviewState {
    type Err = Error;

    fn from_str(s: &str) -> Result<Self> {
        match s {
            "open" => Ok(ReviewState::Open),
            "merged" => Ok(ReviewState::Merged),
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
}

fn review_from_entry(entry: &LedgerEntry, display_id: Option<String>) -> Result<Review> {
    let mut title = String::new();
    let mut state = ReviewState::Open;
    let mut description = String::new();
    let mut source_ref: Option<String> = None;
    let mut head = String::new();
    let mut base: Option<String> = None;

    for (name, value) in &entry.fields {
        let text = || String::from_utf8_lossy(value).into_owned();
        match name.as_str() {
            "title" => title = text(),
            "description" => description = text(),
            "meta/state" => state = text().parse()?,
            "meta/ref" => source_ref = Some(text()),
            "meta/target/head" => head = text(),
            "meta/target/base" => base = Some(text()),
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
    })
}

/// Pin target objects into the `objects/` subtree of the entity.
///
/// Returns `(field_name, oid_bytes)` pairs for each object to pin.
fn pin_fields(repo: &Repository, target: &ReviewTarget) -> Result<Vec<(String, Vec<u8>)>> {
    let mut fields = Vec::new();
    let head_oid = git2::Oid::from_str(&target.head)?;
    let head_obj = repo.find_object(head_oid, None)?;
    let head_mode = object_file_mode(head_obj.kind());
    fields.push((
        format!("objects/{}", target.head),
        mode_tagged_bytes(head_mode),
    ));
    if let Some(ref base) = target.base {
        let base_oid = git2::Oid::from_str(base)?;
        let base_obj = repo.find_object(base_oid, None)?;
        let base_mode = object_file_mode(base_obj.kind());
        fields.push((format!("objects/{base}"), mode_tagged_bytes(base_mode)));
    }
    Ok(fields)
}

/// Map a git object type to the tree entry file mode used in the `objects/` tree.
fn object_file_mode(kind: Option<ObjectType>) -> FileMode {
    match kind {
        Some(ObjectType::Tree) => FileMode::Tree,
        Some(ObjectType::Commit) => FileMode::Link, // gitlink / 160000
        _ => FileMode::Blob,
    }
}

/// Produce a zero-length byte vec — the actual content is irrelevant for pin
/// entries since the OID is in the tree entry name.
fn mode_tagged_bytes(_mode: FileMode) -> Vec<u8> {
    Vec::new()
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

        Ok(Review {
            oid: entry.id,
            display_id: None,
            title: title.to_string(),
            target: target.clone(),
            source_ref: source_ref.map(String::from),
            state: ReviewState::Open,
            description: description.to_string(),
        })
    }

    /// Create a review with a custom git author, used when importing from an external source.
    ///
    /// `display_id` is written to the index immediately.
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
        display_id: &str,
        author: &git2::Signature<'_>,
        source: &str,
    ) -> Result<Review> {
        let mut fields: Vec<(&str, &[u8])> = vec![
            ("title", title.as_bytes()),
            ("description", description.as_bytes()),
            ("meta/state", b"open"),
            ("meta/target/head", target.head.as_bytes()),
            ("source/url", source.as_bytes()),
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
            "forge: create review",
            Some(author),
        )?;

        index_upsert(self.repo, REVIEW_INDEX, &[(display_id, &entry.id)])?;

        Ok(Review {
            oid: entry.id.clone(),
            display_id: Some(display_id.to_string()),
            title: title.to_string(),
            target: target.clone(),
            source_ref: source_ref.map(String::from),
            state: ReviewState::Open,
            description: description.to_string(),
        })
    }

    /// Create a review with a custom git author, skipping object pinning.
    ///
    /// Used when importing from an external source where target objects may not
    /// be available in the local repository (e.g. PR head commits not yet fetched).
    ///
    /// # Errors
    /// Returns an error if a git operation fails.
    #[allow(clippy::too_many_arguments)]
    pub fn create_review_imported_no_pin(
        &self,
        title: &str,
        description: &str,
        target: &ReviewTarget,
        source_ref: Option<&str>,
        display_id: &str,
        author: &git2::Signature<'_>,
        source: &str,
    ) -> Result<Review> {
        let mut fields: Vec<(&str, &[u8])> = vec![
            ("title", title.as_bytes()),
            ("description", description.as_bytes()),
            ("meta/state", b"open"),
            ("meta/target/head", target.head.as_bytes()),
            ("source/url", source.as_bytes()),
        ];
        if let Some(ref base) = target.base {
            fields.push(("meta/target/base", base.as_bytes()));
        }
        if let Some(sref) = source_ref {
            fields.push(("meta/ref", sref.as_bytes()));
        }

        let entry = self.repo.create(
            REVIEW_PREFIX,
            &IdStrategy::CommitOid,
            &fields,
            "forge: create review",
            Some(author),
        )?;

        index_upsert(self.repo, REVIEW_INDEX, &[(display_id, &entry.id)])?;

        Ok(Review {
            oid: entry.id.clone(),
            display_id: Some(display_id.to_string()),
            title: title.to_string(),
            target: target.clone(),
            source_ref: source_ref.map(String::from),
            state: ReviewState::Open,
            description: description.to_string(),
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
        review_from_entry(&entry, display_id)
    }
}
