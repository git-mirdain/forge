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
//! │   └── <actor-uuid>   # empty blob
//! ├── target/
//! │   ├── head         # blob: <oid>
//! │   └── base         # blob: <oid> (optional)
//! ├── objects/
//! │   └── <oid>        # actual git object (pin for GC)
//! └── approvals/
//!     └── <oid>/
//!         └── <actor-uuid>   # empty blob
//! ```
//!
//! Authorship and timestamps live in the commit metadata.

use facet::Facet;
use git_ledger::{FileMode, IdStrategy, Ledger, LedgerEntry, Mutation};
use git2::{ObjectType, Repository};
use serde::Serialize;

use crate::index::{display_id_for_oid, index_upsert, read_index, resolve_oid};
use crate::refs::{APPROVALS_INDEX, REVIEW_INDEX, REVIEW_PREFIX};
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
    /// Workspace-relative path this review is scoped to (e.g. `"crates/git-forge"`).
    /// When set, `retarget` with no explicit head resolves `HEAD:<path>` automatically.
    pub path: Option<String>,
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
    let mut path: Option<String> = None;
    let mut labels = Vec::new();
    let mut assignees = Vec::new();
    // approvals/<oid>/<actor-uuid> — collect all entries
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
            "target/path" => path = Some(text()),
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
        target: ReviewTarget { head, base, path },
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

/// Enumerate the `Mutation::Pin` entries for a review's `objects/` subtree.
///
/// For a commit range (`base..head`), yields every commit reachable from
/// `head` that is not reachable from `base`.  For a single object, yields
/// just that object.  Silently omits objects not present locally.
/// Rebuild the `refs/forge/index/approvals-by-oid` derived index.
///
/// Walks every review, expands each approved OID to its constituent blobs
/// (tree and commit approvals flatten recursively), and writes the result as
/// a fanout tree: `<oid[0..2]>/<oid[2..]>` → newline-separated approver UUIDs.
fn rebuild_approvals_index(repo: &Repository) -> Result<()> {
    use std::collections::{BTreeMap, BTreeSet};

    let known_oids = repo.list(REVIEW_PREFIX)?;
    let mut by_blob: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();

    for ref_oid in &known_oids {
        let ref_name = format!("{REVIEW_PREFIX}{ref_oid}");
        let Ok(entry) = repo.read(&ref_name) else {
            continue;
        };
        let Ok(review) = review_from_entry(repo, &entry, &ref_name, None) else {
            continue;
        };
        for approval in review.approvals {
            if approval.approvers.is_empty() {
                continue;
            }
            let Ok(oid) = git2::Oid::from_str(&approval.oid) else {
                continue;
            };
            for blob_oid in collect_blobs(repo, oid) {
                let entry = by_blob.entry(blob_oid).or_default();
                for approver in &approval.approvers {
                    entry.insert(approver.clone());
                }
            }
        }
    }

    let mut root_builder = repo.treebuilder(None)?;
    let mut by_prefix: BTreeMap<String, BTreeMap<String, BTreeSet<String>>> = BTreeMap::new();
    for (oid, approvers) in &by_blob {
        if oid.len() < 3 {
            continue;
        }
        let (prefix, rest) = oid.split_at(2);
        by_prefix
            .entry(prefix.to_string())
            .or_default()
            .insert(rest.to_string(), approvers.clone());
    }
    for (prefix, entries) in &by_prefix {
        let mut dir_builder = repo.treebuilder(None)?;
        for (rest, approvers) in entries {
            let content: String = approvers.iter().fold(String::new(), |mut acc, a| {
                acc.push_str(a);
                acc.push('\n');
                acc
            });
            let blob_oid = repo.blob(content.as_bytes())?;
            dir_builder.insert(rest, blob_oid, 0o100_644)?;
        }
        let dir_oid = dir_builder.write()?;
        root_builder.insert(prefix, dir_oid, 0o040_000)?;
    }

    let root_tree_oid = root_builder.write()?;
    let root_tree = repo.find_tree(root_tree_oid)?;
    let sig = repo.signature()?;
    let parent = repo
        .find_reference(APPROVALS_INDEX)
        .ok()
        .and_then(|r| r.peel_to_commit().ok());
    let parents: Vec<&git2::Commit<'_>> = parent.iter().collect();
    repo.commit(
        Some(APPROVALS_INDEX),
        &sig,
        &sig,
        "forge: rebuild approvals-by-oid index",
        &root_tree,
        &parents,
    )?;
    Ok(())
}

/// Recursively collect all blob OIDs reachable from `oid`.
///
/// - Blob → `[oid]`
/// - Tree → all blobs in the tree, recursively
/// - Commit → blobs reachable from the commit's tree
///
/// Any object that cannot be resolved is silently skipped.
fn collect_blobs(repo: &Repository, oid: git2::Oid) -> Vec<String> {
    let Ok(obj) = repo.find_object(oid, None) else {
        return vec![];
    };
    match obj.kind() {
        Some(ObjectType::Blob) => vec![oid.to_string()],
        Some(ObjectType::Tree) => {
            let Ok(tree) = repo.find_tree(oid) else {
                return vec![];
            };
            let mut blobs = Vec::new();
            collect_blobs_from_tree(repo, &tree, &mut blobs);
            blobs
        }
        Some(ObjectType::Commit) => {
            let Ok(commit) = repo.find_commit(oid) else {
                return vec![];
            };
            let Ok(tree) = commit.tree() else {
                return vec![];
            };
            let mut blobs = Vec::new();
            collect_blobs_from_tree(repo, &tree, &mut blobs);
            blobs
        }
        _ => vec![],
    }
}

fn collect_blobs_from_tree(repo: &Repository, tree: &git2::Tree<'_>, out: &mut Vec<String>) {
    for entry in tree {
        match entry.kind() {
            Some(ObjectType::Blob) => out.push(entry.id().to_string()),
            Some(ObjectType::Tree) => {
                if let Ok(subtree) = repo.find_tree(entry.id()) {
                    collect_blobs_from_tree(repo, &subtree, out);
                }
            }
            _ => {}
        }
    }
}

fn enumerate_pins(
    repo: &Repository,
    target: &ReviewTarget,
) -> Result<Vec<(String, git2::Oid, FileMode)>> {
    let head_oid = git2::Oid::from_str(&target.head)?;

    if let Some(base_str) = &target.base {
        let base_oid = git2::Oid::from_str(base_str)?;
        let mut walk = repo.revwalk()?;
        if walk.push(head_oid).is_err() {
            return Ok(vec![]);
        }
        // hide is best-effort: if base doesn't exist locally, walk from head only
        let _ = walk.hide(base_oid);
        Ok(walk
            .flatten()
            .map(|oid| {
                let mode = object_mode(repo, oid);
                (oid.to_string(), oid, mode)
            })
            .collect())
    } else {
        let Ok(obj) = repo.find_object(head_oid, None) else {
            return Ok(vec![]);
        };
        Ok(vec![(
            target.head.clone(),
            head_oid,
            object_mode_from_type(obj.kind()),
        )])
    }
}

fn object_mode(repo: &Repository, oid: git2::Oid) -> FileMode {
    object_mode_from_type(repo.find_object(oid, None).ok().and_then(|o| o.kind()))
}

fn object_mode_from_type(kind: Option<ObjectType>) -> FileMode {
    match kind {
        Some(ObjectType::Tree) => FileMode::Tree,
        Some(ObjectType::Commit) => FileMode::Commit,
        _ => FileMode::Blob,
    }
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
        author: Option<&git2::Signature<'_>>,
    ) -> Result<Review> {
        let pins = enumerate_pins(self.repo, target)?;
        let pin_paths: Vec<String> = pins
            .iter()
            .map(|(s, _, _)| format!("objects/{s}"))
            .collect();
        let objects: Vec<String> = pins.iter().map(|(s, _, _)| s.clone()).collect();

        let mut mutations: Vec<Mutation<'_>> = vec![
            Mutation::Set("title", title.as_bytes()),
            Mutation::Set("body", body.as_bytes()),
            Mutation::Set("state", b"open"),
            Mutation::Set("target/head", target.head.as_bytes()),
        ];
        if let Some(ref base) = target.base {
            mutations.push(Mutation::Set("target/base", base.as_bytes()));
        }
        if let Some(sref) = source_ref {
            mutations.push(Mutation::Set("source_ref", sref.as_bytes()));
        }
        for ((_, oid, mode), path) in pins.iter().zip(pin_paths.iter()) {
            mutations.push(Mutation::Pin(path.as_str(), *oid, *mode));
        }

        let entry = self.repo.create(
            REVIEW_PREFIX,
            &IdStrategy::CommitOid,
            &mutations,
            "create review",
            author,
        )?;

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
        let pins = enumerate_pins(self.repo, target)?;
        let pin_paths: Vec<String> = pins
            .iter()
            .map(|(s, _, _)| format!("objects/{s}"))
            .collect();
        let objects: Vec<String> = pins.iter().map(|(s, _, _)| s.clone()).collect();

        let mut mutations: Vec<Mutation<'_>> = vec![
            Mutation::Set("title", title.as_bytes()),
            Mutation::Set("body", body.as_bytes()),
            Mutation::Set("state", state_str.as_bytes()),
            Mutation::Set("target/head", target.head.as_bytes()),
            Mutation::Set("source/url", source.as_bytes()),
        ];
        if let Some(ref base) = target.base {
            mutations.push(Mutation::Set("target/base", base.as_bytes()));
        }
        if let Some(sref) = source_ref {
            mutations.push(Mutation::Set("source_ref", sref.as_bytes()));
        }
        for ((_, oid, mode), path) in pins.iter().zip(pin_paths.iter()) {
            mutations.push(Mutation::Pin(path.as_str(), *oid, *mode));
        }

        let entry = self.repo.create(
            REVIEW_PREFIX,
            &IdStrategy::CommitOid,
            &mutations,
            "forge: create review",
            Some(author),
        )?;

        let oid = entry.id.clone();
        index_upsert(self.repo, REVIEW_INDEX, &[(display_id, &oid)])?;

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

    /// Rebuild the review display-ID index from scratch.
    ///
    /// Same logic as [`Store::reindex_issues`] but for reviews — parses
    /// `source/url` fields matching `…/pull/{number}` and applies the
    /// current `review` sigil from config.
    ///
    /// Returns the number of entries written.
    ///
    /// # Errors
    /// Returns an error if any git operation fails.
    pub fn reindex_reviews(&self) -> Result<usize> {
        use crate::refs;

        let old_index = read_index(self.repo, REVIEW_INDEX)?;
        let mut oids = self.repo.list(REVIEW_PREFIX)?;
        oids.sort();

        let sigil_map = crate::reindex::build_sigil_map(self.repo, "review")?;

        let mut entries: Vec<(String, String)> = Vec::new();
        let mut used_ids: std::collections::HashSet<String> = std::collections::HashSet::new();
        let mut next_local_id = 1u64;

        for oid in &oids {
            let ref_name = format!("{REVIEW_PREFIX}{oid}");
            let entry = self.repo.read(&ref_name)?;

            if let Some(display_id) =
                crate::reindex::display_id_from_source(&entry, &sigil_map, "pull")
            {
                used_ids.insert(display_id.clone());
                entries.push((display_id, oid.clone()));
            } else {
                let existing = old_index
                    .as_ref()
                    .and_then(|idx| idx.iter().find(|(_, v)| v.as_str() == oid))
                    .map(|(k, _)| k.clone());
                let display_id = if let Some(id) = existing {
                    used_ids.insert(id.clone());
                    id
                } else {
                    while used_ids.contains(&next_local_id.to_string()) {
                        next_local_id += 1;
                    }
                    let id = next_local_id.to_string();
                    used_ids.insert(id.clone());
                    next_local_id += 1;
                    id
                };
                entries.push((display_id, oid.clone()));
            }
        }

        let count = entries.len();
        let pairs: Vec<(&str, &str)> = entries
            .iter()
            .map(|(k, v)| (k.as_str(), v.as_str()))
            .collect();
        crate::reindex::write_index_from_scratch(self.repo, refs::REVIEW_INDEX, &pairs)?;
        Ok(count)
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
    //
    // TOCTOU: read-then-write is not atomic. A concurrent writer can
    // interleave between the `read` and the `update`, causing one write to
    // silently overwrite the other. The intended fix is optimistic
    // concurrency via an expected-parent-OID check at the ledger layer.
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

        if mutations.is_empty() {
            let entry = self.repo.read(&ref_name)?;
            let display_id = display_id_for_oid(index.as_ref(), &oid);
            return review_from_entry(self.repo, &entry, &ref_name, display_id);
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
            path: review.target.path.clone(),
        };
        let pins = enumerate_pins(self.repo, &new_target)?;
        let pin_paths: Vec<String> = pins
            .iter()
            .map(|(s, _, _)| format!("objects/{s}"))
            .collect();

        let mut mutations: Vec<Mutation<'_>> =
            vec![Mutation::Set("target/head", new_head.as_bytes())];
        for ((_, oid, mode), path) in pins.iter().zip(pin_paths.iter()) {
            mutations.push(Mutation::Pin(path.as_str(), *oid, *mode));
        }

        let entry = self
            .repo
            .update(&ref_name, &mutations, "refresh review target")?;
        review_from_entry(self.repo, &entry, &ref_name, display_id)
    }

    /// Retarget a review to a new head OID, updating `objects/`.
    ///
    /// Returns `(old_head, updated_review)`.
    ///
    /// # Errors
    /// Returns an error if the review does not exist or a git operation fails.
    pub fn retarget_review(
        &self,
        oid_or_id: &str,
        new_head: &str,
        new_path: Option<&str>,
    ) -> Result<(String, Review)> {
        let index = read_index(self.repo, REVIEW_INDEX)?;
        let known_oids = self.repo.list(REVIEW_PREFIX)?;
        let oid = resolve_oid(index.as_ref(), &known_oids, oid_or_id)?;
        let ref_name = format!("{REVIEW_PREFIX}{oid}");
        let entry = self.repo.read(&ref_name)?;
        let display_id = display_id_for_oid(index.as_ref(), &oid);
        let old_review = review_from_entry(self.repo, &entry, &ref_name, display_id.clone())?;
        let old_head = old_review.target.head.clone();

        let path = new_path
            .map(str::to_string)
            .or(old_review.target.path.clone());

        let new_target = ReviewTarget {
            head: new_head.to_string(),
            base: old_review.target.base.clone(),
            path: path.clone(),
        };
        let pins = enumerate_pins(self.repo, &new_target)?;
        let new_object_set: std::collections::HashSet<&str> =
            pins.iter().map(|(s, _, _)| s.as_str()).collect();
        let pin_paths: Vec<String> = pins
            .iter()
            .map(|(s, _, _)| format!("objects/{s}"))
            .collect();

        let mut mutations: Vec<Mutation<'_>> =
            vec![Mutation::Set("target/head", new_head.as_bytes())];
        if let Some(ref p) = path {
            mutations.push(Mutation::Set("target/path", p.as_bytes()));
        }
        // Delete stale object pins that are not in the new set.
        let stale_paths: Vec<String> = old_review
            .objects
            .iter()
            .filter(|o| !new_object_set.contains(o.as_str()))
            .map(|o| format!("objects/{o}"))
            .collect();
        for p in &stale_paths {
            mutations.push(Mutation::Delete(p.as_str()));
        }
        for ((_, oid, mode), path) in pins.iter().zip(pin_paths.iter()) {
            mutations.push(Mutation::Pin(path.as_str(), *oid, *mode));
        }

        let entry = self.repo.update(&ref_name, &mutations, "retarget review")?;
        let review = review_from_entry(self.repo, &entry, &ref_name, display_id)?;
        Ok((old_head, review))
    }

    /// Approve all objects in a review as the given actor UUID.
    ///
    /// Writes `approvals/<oid>/<actor-uuid>` for each object in `objects/`.
    ///
    /// # Errors
    /// Returns an error if the review does not exist or a git operation fails.
    pub fn approve_review(&self, oid_or_id: &str, actor_uuid: &str) -> Result<Review> {
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
            .map(|obj_oid| format!("approvals/{obj_oid}/{actor_uuid}"))
            .collect();
        let mutations: Vec<Mutation<'_>> = approval_paths
            .iter()
            .map(|p| Mutation::Set(p.as_str(), b""))
            .collect();

        let entry = self.repo.update(&ref_name, &mutations, "approve review")?;
        rebuild_approvals_index(self.repo)?;
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
        actor_uuid: &str,
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

        let field = format!("approvals/{obj_oid}/{actor_uuid}");
        let mutations = [Mutation::Set(field.as_str(), b"")];
        let entry = self
            .repo
            .update(&ref_name, &mutations, "approve review object")?;
        let display_id = display_id_for_oid(index.as_ref(), &oid);
        review_from_entry(self.repo, &entry, &ref_name, display_id)
    }

    /// Revoke approval of all objects for an actor.
    ///
    /// # Errors
    /// Returns an error if the review does not exist.
    pub fn revoke_approval(&self, oid_or_id: &str, actor_uuid: &str) -> Result<Review> {
        let index = read_index(self.repo, REVIEW_INDEX)?;
        let known_oids = self.repo.list(REVIEW_PREFIX)?;
        let oid = resolve_oid(index.as_ref(), &known_oids, oid_or_id)?;
        let ref_name = format!("{REVIEW_PREFIX}{oid}");

        let entry = self.repo.read(&ref_name)?;
        let review = review_from_entry(self.repo, &entry, &ref_name, None)?;

        let del_paths: Vec<String> = review
            .objects
            .iter()
            .map(|obj_oid| format!("approvals/{obj_oid}/{actor_uuid}"))
            .collect();
        let mutations: Vec<Mutation<'_>> = del_paths
            .iter()
            .map(|p| Mutation::Delete(p.as_str()))
            .collect();

        let entry = self.repo.update(&ref_name, &mutations, "revoke approval")?;
        rebuild_approvals_index(self.repo)?;
        let display_id = display_id_for_oid(index.as_ref(), &oid);
        review_from_entry(self.repo, &entry, &ref_name, display_id)
    }

    /// Return the set of blob OIDs that have at least one approval across all
    /// reviews. Reads from the `refs/forge/index/approvals-by-oid` derived
    /// index; falls back to a full review scan if the index is absent.
    ///
    /// # Errors
    /// Returns an error if a git operation fails.
    pub fn approved_oids(&self) -> Result<std::collections::HashSet<String>> {
        use git2::ErrorCode;

        let reference = match self.repo.find_reference(APPROVALS_INDEX) {
            Ok(r) => r,
            Err(e) if e.code() == ErrorCode::NotFound => {
                // Index not yet built; fall back to full scan.
                let reviews = self.list_reviews()?;
                let mut set = std::collections::HashSet::new();
                for review in reviews {
                    for entry in review.approvals {
                        if !entry.approvers.is_empty() {
                            let Ok(oid) = git2::Oid::from_str(&entry.oid) else {
                                continue;
                            };
                            for blob_oid in collect_blobs(self.repo, oid) {
                                set.insert(blob_oid);
                            }
                        }
                    }
                }
                return Ok(set);
            }
            Err(e) => return Err(e.into()),
        };

        let tree = reference.peel_to_commit()?.tree()?;
        let mut set = std::collections::HashSet::new();
        for dir_entry in &tree {
            let Some(prefix) = dir_entry.name() else {
                continue;
            };
            if dir_entry.kind() != Some(ObjectType::Tree) {
                continue;
            }
            let dir_tree = self.repo.find_tree(dir_entry.id())?;
            for leaf in &dir_tree {
                let Some(rest) = leaf.name() else {
                    continue;
                };
                set.insert(format!("{prefix}{rest}"));
            }
        }
        Ok(set)
    }
}
