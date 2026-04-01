//! Review entity CRUD backed by `git-ledger`.
//!
//! Tree layout at `refs/forge/reviews/<uuid-v7>`:
//!
//! ```text
//! ├── title
//! ├── body
//! ├── state            # "open" | "draft" | "closed" | "merged"
//! ├── labels/
//! │   └── <name>       # empty blob
//! ├── assignees/
//! │   └── <contributor-uuid>   # empty blob
//! ├── target/
//! │   ├── head         # blob: <oid>
//! │   └── base         # blob: <oid> (optional)
//! ├── objects/
//! │   └── <oid>        # actual git object (pin for GC)
//! └── approvals/
//!     └── <oid>/
//!         └── <contributor-uuid>   # empty blob
//! ```
//!
//! Authorship and timestamps live in the commit metadata.

use facet::Facet;
use git_ledger::{IdStrategy, Ledger, LedgerEntry, Mutation};
use git2::{FileMode, ObjectType, Repository};
use serde::Serialize;

use crate::index::{display_id_for_oid, index_upsert, read_index, resolve_oid};
use crate::refs::{REVIEW_INDEX, REVIEW_PREFIX};
use crate::{Error, Result, Store};

// ── state ─────────────────────────────────────────────────────────────────────

/// The lifecycle state of a review.
#[derive(Debug, Clone, Serialize, Facet, PartialEq, Eq)]
#[repr(u8)]
#[serde(rename_all = "lowercase")]
pub enum ReviewState {
    /// Review is open and accepting comments and approvals.
    Open,
    /// Draft — not ready for review; assignees are not expected to act.
    Draft,
    /// Closed without merging.
    Closed,
    /// Terminal: the target was incorporated.
    Merged,
}

impl ReviewState {
    /// Return the canonical string representation.
    #[must_use]
    pub fn as_str(&self) -> &'static str {
        match self {
            ReviewState::Open => "open",
            ReviewState::Draft => "draft",
            ReviewState::Closed => "closed",
            ReviewState::Merged => "merged",
        }
    }
}

impl std::str::FromStr for ReviewState {
    type Err = Error;

    fn from_str(s: &str) -> Result<Self> {
        match s {
            "open" => Ok(ReviewState::Open),
            "draft" => Ok(ReviewState::Draft),
            "closed" => Ok(ReviewState::Closed),
            "merged" => Ok(ReviewState::Merged),
            _ => Err(Error::InvalidState(s.to_string())),
        }
    }
}

// ── types ─────────────────────────────────────────────────────────────────────

/// The target objects for a review.
#[derive(Debug, Clone, Serialize, Facet)]
pub struct ReviewTarget {
    /// The head object OID (required).
    pub head: String,
    /// The base object OID (optional — absent for single-object reviews).
    pub base: Option<String>,
}

/// Per-OID approval coverage within a review.
#[derive(Debug, Clone, Serialize, Facet)]
pub struct ApprovalEntry {
    /// The object OID being approved.
    pub oid: String,
    /// Contributor UUIDs that have approved this object.
    pub approvers: Vec<String>,
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
    /// Body in Markdown.
    pub body: String,
    /// Label names attached to this review.
    pub labels: Vec<String>,
    /// Contributor UUIDs assigned to this review.
    pub assignees: Vec<String>,
    /// Objects kept reachable for GC.
    pub objects: Vec<String>,
    /// Per-OID approval entries.
    pub approvals: Vec<ApprovalEntry>,
}

// ── internal ──────────────────────────────────────────────────────────────────

fn review_from_entry(
    repo: &Repository,
    entry: &LedgerEntry,
    ref_name: &str,
    display_id: Option<String>,
) -> Result<Review> {
    let mut title = String::new();
    let mut body = String::new();
    let mut state = ReviewState::Open;
    let mut source_ref: Option<String> = None;
    let mut head = String::new();
    let mut base: Option<String> = None;
    let mut labels = Vec::new();
    let mut assignees = Vec::new();
    // approvals/<oid>/<contributor-uuid> — collect all entries
    let mut approvals_map: std::collections::BTreeMap<String, Vec<String>> =
        std::collections::BTreeMap::new();

    for (name, value) in &entry.fields {
        let text = || String::from_utf8_lossy(value).into_owned();
        match name.as_str() {
            "title" => title = text(),
            "body" => body = text(),
            "state" => state = text().parse()?,
            "source_ref" => source_ref = Some(text()),
            "target/head" => head = text(),
            "target/base" => base = Some(text()),
            n if n.starts_with("labels/") => {
                labels.push(n["labels/".len()..].to_string());
            }
            n if n.starts_with("assignees/") => {
                assignees.push(n["assignees/".len()..].to_string());
            }
            n if n.starts_with("approvals/") => {
                let rest = &n["approvals/".len()..];
                if let Some((oid, contributor)) = rest.split_once('/') {
                    approvals_map
                        .entry(oid.to_string())
                        .or_default()
                        .push(contributor.to_string());
                }
            }
            _ => {}
        }
    }

    // Read `objects/` directly from the tree: entry names are OIDs regardless
    // of mode (blobs, commits, trees all qualify).
    let objects = read_objects_subtree(repo, ref_name);

    let approvals = approvals_map
        .into_iter()
        .map(|(oid, approvers)| ApprovalEntry { oid, approvers })
        .collect();

    Ok(Review {
        oid: entry.id.clone(),
        display_id,
        title,
        target: ReviewTarget { head, base },
        source_ref,
        state,
        body,
        labels,
        assignees,
        objects,
        approvals,
    })
}

/// Read entry names from the `objects/` subtree of the latest commit on
/// `ref_name`.  Returns an empty vec if the subtree doesn't exist.
fn read_objects_subtree(repo: &Repository, ref_name: &str) -> Vec<String> {
    let Ok(reference) = repo.find_reference(ref_name) else {
        return Vec::new();
    };
    let Ok(commit) = reference.peel_to_commit() else {
        return Vec::new();
    };
    let Ok(tree) = commit.tree() else {
        return Vec::new();
    };
    let Some(entry) = tree.get_name("objects") else {
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

/// Enumerate the objects that `target` implies for the `objects/` manifest.
///
/// For a commit range (`base..head`), returns every commit reachable from
/// `head` that is not reachable from `base`.  For a single object, returns
/// just the head OID.  Silently omits objects not present locally.
fn enumerate_objects(repo: &Repository, target: &ReviewTarget) -> Vec<String> {
    let Some(head_oid) = git2::Oid::from_str(&target.head).ok() else {
        return Vec::new();
    };

    match &target.base {
        Some(base_str) => {
            // Commit range: enumerate commits in base..head.
            let Some(base_oid) = git2::Oid::from_str(base_str).ok() else {
                return vec![target.head.clone()];
            };
            let Ok(mut walk) = repo.revwalk() else {
                return vec![target.head.clone()];
            };
            if walk.push(head_oid).is_err() || walk.hide(base_oid).is_err() {
                return vec![target.head.clone()];
            }
            walk.filter_map(|r| r.ok().map(|id| id.to_string()))
                .collect()
        }
        None => {
            if repo.find_object(head_oid, None).is_ok() {
                vec![target.head.clone()]
            } else {
                Vec::new()
            }
        }
    }
}

/// Map a git object type to the tree entry file mode for the `objects/` tree.
fn object_file_mode(kind: Option<ObjectType>) -> FileMode {
    match kind {
        Some(ObjectType::Tree) => FileMode::Tree,
        Some(ObjectType::Commit) => FileMode::Commit,
        _ => FileMode::Blob,
    }
}

/// Rewrite the `objects/` subtree so each entry's OID points to the actual
/// git object rather than the placeholder blob created by `Ledger::create`.
fn fixup_pin_entries(repo: &Repository, review_oid: &str, objects: &[String]) -> Result<()> {
    if objects.is_empty() {
        return Ok(());
    }

    let ref_name = format!("{REVIEW_PREFIX}{review_oid}");
    let reference = repo.find_reference(&ref_name)?;
    let commit = reference.peel_to_commit()?;
    let root_tree = commit.tree()?;

    let existing_objects = root_tree.get_name("objects").and_then(|e| {
        if e.kind() == Some(ObjectType::Tree) {
            repo.find_tree(e.id()).ok()
        } else {
            None
        }
    });
    let mut obj_builder = repo.treebuilder(existing_objects.as_ref())?;

    for oid_str in objects {
        let Ok(oid) = git2::Oid::from_str(oid_str) else {
            continue;
        };
        let Ok(obj) = repo.find_object(oid, None) else {
            continue;
        };
        obj_builder.insert(oid_str, oid, i32::from(object_file_mode(obj.kind())))?;
    }

    let obj_tree_oid = obj_builder.write()?;
    let mut root_builder = repo.treebuilder(Some(&root_tree))?;
    root_builder.insert("objects", obj_tree_oid, 0o040_000)?;
    let new_tree_oid = root_builder.write()?;
    let new_tree = repo.find_tree(new_tree_oid)?;

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
    repo.reference(&ref_name, new_commit, true, "fixup pin entries")?;
    Ok(())
}

// ── Store impl ────────────────────────────────────────────────────────────────

impl Store<'_> {
    /// Create a new review.
    ///
    /// # Errors
    /// Returns an error if a git operation fails.
    pub fn create_review(
        &self,
        title: &str,
        body: &str,
        target: &ReviewTarget,
        source_ref: Option<&str>,
    ) -> Result<Review> {
        let objects = enumerate_objects(self.repo, target);

        let mut fields: Vec<(&str, &[u8])> = vec![
            ("title", title.as_bytes()),
            ("body", body.as_bytes()),
            ("state", b"open"),
            ("target/head", target.head.as_bytes()),
        ];
        if let Some(ref base) = target.base {
            fields.push(("target/base", base.as_bytes()));
        }
        if let Some(sref) = source_ref {
            fields.push(("source_ref", sref.as_bytes()));
        }

        let obj_paths: Vec<String> = objects.iter().map(|o| format!("objects/{o}")).collect();
        let obj_fields: Vec<(&str, &[u8])> = obj_paths
            .iter()
            .map(|p| (p.as_str(), b"" as &[u8]))
            .collect();
        fields.extend(obj_fields);

        let entry = self.repo.create(
            REVIEW_PREFIX,
            &IdStrategy::CommitOid,
            &fields,
            "create review",
            None,
        )?;

        fixup_pin_entries(self.repo, &entry.id, &objects)?;

        Ok(Review {
            oid: entry.id,
            display_id: None,
            title: title.to_string(),
            target: target.clone(),
            source_ref: source_ref.map(String::from),
            state: ReviewState::Open,
            body: body.to_string(),
            labels: Vec::new(),
            assignees: Vec::new(),
            objects,
            approvals: Vec::new(),
        })
    }

    /// Create a review from an external source with a custom author.
    ///
    /// `display_id` is written to the index immediately. Objects that are not
    /// locally available are skipped when building the `objects/` manifest.
    ///
    /// # Errors
    /// Returns an error if a git operation fails.
    #[allow(clippy::too_many_arguments)]
    pub fn create_review_imported(
        &self,
        title: &str,
        body: &str,
        target: &ReviewTarget,
        source_ref: Option<&str>,
        state: Option<&ReviewState>,
        display_id: &str,
        author: &git2::Signature<'_>,
        source: &str,
    ) -> Result<Review> {
        let state = state.cloned().unwrap_or(ReviewState::Open);
        let state_str = state.as_str().to_string();
        let objects = enumerate_objects(self.repo, target);

        let mut fields: Vec<(&str, &[u8])> = vec![
            ("title", title.as_bytes()),
            ("body", body.as_bytes()),
            ("state", state_str.as_bytes()),
            ("target/head", target.head.as_bytes()),
            ("source/url", source.as_bytes()),
        ];
        if let Some(ref base) = target.base {
            fields.push(("target/base", base.as_bytes()));
        }
        if let Some(sref) = source_ref {
            fields.push(("source_ref", sref.as_bytes()));
        }

        let obj_paths: Vec<String> = objects.iter().map(|o| format!("objects/{o}")).collect();
        let obj_fields: Vec<(&str, &[u8])> = obj_paths
            .iter()
            .map(|p| (p.as_str(), b"" as &[u8]))
            .collect();
        fields.extend(obj_fields);

        let entry = self.repo.create(
            REVIEW_PREFIX,
            &IdStrategy::CommitOid,
            &fields,
            "forge: create review",
            Some(author),
        )?;

        let oid = entry.id.clone();
        index_upsert(self.repo, REVIEW_INDEX, &[(display_id, &oid)])?;
        fixup_pin_entries(self.repo, &oid, &objects)?;

        Ok(Review {
            oid,
            display_id: Some(display_id.to_string()),
            title: title.to_string(),
            target: target.clone(),
            source_ref: source_ref.map(String::from),
            state,
            body: body.to_string(),
            labels: Vec::new(),
            assignees: Vec::new(),
            objects,
            approvals: Vec::new(),
        })
    }

    /// Fetch a single review by display ID or OID prefix.
    ///
    /// # Errors
    /// Returns [`Error::NotFound`] if the review does not exist.
    pub fn get_review(&self, oid_or_id: &str) -> Result<Review> {
        let index = read_index(self.repo, REVIEW_INDEX)?;
        let known_oids = self.repo.list(REVIEW_PREFIX)?;
        let oid = resolve_oid(index.as_ref(), &known_oids, oid_or_id)?;
        let ref_name = format!("{REVIEW_PREFIX}{oid}");
        let entry = self.repo.read(&ref_name)?;
        let display_id = display_id_for_oid(index.as_ref(), &oid);
        review_from_entry(self.repo, &entry, &ref_name, display_id)
    }

    /// List all reviews.
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
                review_from_entry(self.repo, &entry, &ref_name, display_id)
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
    /// Returns [`Error::NotFound`] if the review does not exist.
    pub fn update_review(
        &self,
        oid_or_id: &str,
        title: Option<&str>,
        body: Option<&str>,
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
        if let Some(b) = body {
            mutations.push(Mutation::Set("body", b.as_bytes()));
        }
        if let Some(ref s) = state_bytes {
            mutations.push(Mutation::Set("state", s.as_bytes()));
        }

        let entry = self.repo.update(&ref_name, &mutations, "update review")?;
        let display_id = display_id_for_oid(index.as_ref(), &oid);
        review_from_entry(self.repo, &entry, &ref_name, display_id)
    }

    /// Re-resolve `source_ref` to update `target/head` and `objects/`.
    ///
    /// No-op if the review has no `source_ref`.
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
        let review = review_from_entry(self.repo, &entry, &ref_name, display_id.clone())?;

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
        let objects = enumerate_objects(self.repo, &new_target);
        let obj_paths: Vec<String> = objects.iter().map(|o| format!("objects/{o}")).collect();

        let mut mutations: Vec<Mutation<'_>> =
            vec![Mutation::Set("target/head", new_head.as_bytes())];
        for p in &obj_paths {
            mutations.push(Mutation::Set(p.as_str(), b""));
        }

        let entry = self
            .repo
            .update(&ref_name, &mutations, "refresh review target")?;
        fixup_pin_entries(self.repo, &oid, &objects)?;
        review_from_entry(self.repo, &entry, &ref_name, display_id)
    }

    /// Retarget a review to a new head OID, updating `objects/`.
    ///
    /// Returns `(old_head, updated_review)`.
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
        let old_review = review_from_entry(self.repo, &entry, &ref_name, display_id.clone())?;
        let old_head = old_review.target.head.clone();

        let new_target = ReviewTarget {
            head: new_head.to_string(),
            base: old_review.target.base.clone(),
        };
        let objects = enumerate_objects(self.repo, &new_target);
        let obj_paths: Vec<String> = objects.iter().map(|o| format!("objects/{o}")).collect();

        let mut mutations: Vec<Mutation<'_>> =
            vec![Mutation::Set("target/head", new_head.as_bytes())];
        for p in &obj_paths {
            mutations.push(Mutation::Set(p.as_str(), b""));
        }

        let entry = self.repo.update(&ref_name, &mutations, "retarget review")?;
        fixup_pin_entries(self.repo, &oid, &objects)?;
        let review = review_from_entry(self.repo, &entry, &ref_name, display_id)?;
        Ok((old_head, review))
    }

    /// Approve all objects in a review as the given contributor UUID.
    ///
    /// Writes `approvals/<oid>/<contributor-uuid>` for each object in `objects/`.
    ///
    /// # Errors
    /// Returns an error if the review does not exist or a git operation fails.
    pub fn approve_review(&self, oid_or_id: &str, contributor_uuid: &str) -> Result<Review> {
        let index = read_index(self.repo, REVIEW_INDEX)?;
        let known_oids = self.repo.list(REVIEW_PREFIX)?;
        let oid = resolve_oid(index.as_ref(), &known_oids, oid_or_id)?;
        let ref_name = format!("{REVIEW_PREFIX}{oid}");

        let entry = self.repo.read(&ref_name)?;
        let review = review_from_entry(self.repo, &entry, &ref_name, None)?;

        if review.objects.is_empty() {
            return Err(Error::Config("review has no objects to approve".into()));
        }

        let approval_paths: Vec<String> = review
            .objects
            .iter()
            .map(|obj_oid| format!("approvals/{obj_oid}/{contributor_uuid}"))
            .collect();
        let mutations: Vec<Mutation<'_>> = approval_paths
            .iter()
            .map(|p| Mutation::Set(p.as_str(), b""))
            .collect();

        let entry = self.repo.update(&ref_name, &mutations, "approve review")?;
        let display_id = display_id_for_oid(index.as_ref(), &oid);
        review_from_entry(self.repo, &entry, &ref_name, display_id)
    }

    /// Approve a single object in a review.
    ///
    /// `obj_oid` must appear in `objects/`.
    ///
    /// # Errors
    /// Returns an error if the review or object is not found.
    pub fn approve_review_object(
        &self,
        oid_or_id: &str,
        obj_oid: &str,
        contributor_uuid: &str,
    ) -> Result<Review> {
        let index = read_index(self.repo, REVIEW_INDEX)?;
        let known_oids = self.repo.list(REVIEW_PREFIX)?;
        let oid = resolve_oid(index.as_ref(), &known_oids, oid_or_id)?;
        let ref_name = format!("{REVIEW_PREFIX}{oid}");

        let entry = self.repo.read(&ref_name)?;
        let review = review_from_entry(self.repo, &entry, &ref_name, None)?;
        if !review.objects.contains(&obj_oid.to_string()) {
            return Err(Error::Config(format!(
                "object {obj_oid} is not in this review's objects"
            )));
        }

        let field = format!("approvals/{obj_oid}/{contributor_uuid}");
        let mutations = [Mutation::Set(field.as_str(), b"")];
        let entry = self
            .repo
            .update(&ref_name, &mutations, "approve review object")?;
        let display_id = display_id_for_oid(index.as_ref(), &oid);
        review_from_entry(self.repo, &entry, &ref_name, display_id)
    }

    /// Revoke approval of all objects for a contributor.
    ///
    /// # Errors
    /// Returns an error if the review does not exist.
    pub fn revoke_approval(&self, oid_or_id: &str, contributor_uuid: &str) -> Result<Review> {
        let index = read_index(self.repo, REVIEW_INDEX)?;
        let known_oids = self.repo.list(REVIEW_PREFIX)?;
        let oid = resolve_oid(index.as_ref(), &known_oids, oid_or_id)?;
        let ref_name = format!("{REVIEW_PREFIX}{oid}");

        let entry = self.repo.read(&ref_name)?;
        let review = review_from_entry(self.repo, &entry, &ref_name, None)?;

        let del_paths: Vec<String> = review
            .objects
            .iter()
            .map(|obj_oid| format!("approvals/{obj_oid}/{contributor_uuid}"))
            .collect();
        let mutations: Vec<Mutation<'_>> = del_paths
            .iter()
            .map(|p| Mutation::Delete(p.as_str()))
            .collect();

        let entry = self.repo.update(&ref_name, &mutations, "revoke approval")?;
        let display_id = display_id_for_oid(index.as_ref(), &oid);
        review_from_entry(self.repo, &entry, &ref_name, display_id)
    }

    /// Return the set of object OIDs that have at least one approval in any
    /// review.
    ///
    /// Scans review refs directly. The `refs/forge/index/approvals-by-oid`
    /// derived index (Phase 5 optimization) is not yet written; this method
    /// is the authoritative fallback.
    ///
    /// # Errors
    /// Returns an error if a git operation fails.
    pub fn approved_oids(&self) -> Result<std::collections::HashSet<String>> {
        let reviews = self.list_reviews()?;
        let mut set = std::collections::HashSet::new();
        for review in reviews {
            for entry in review.approvals {
                if !entry.approvers.is_empty() {
                    set.insert(entry.oid);
                }
            }
        }
        Ok(set)
    }
}
