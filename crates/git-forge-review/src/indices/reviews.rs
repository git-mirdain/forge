//! Review refs under `refs/meta/reviews/`.
//!
//! A review is a coordination entity — "please look at commits X..Y." It
//! references commits but is not metadata on any commit. It has its own
//! lifecycle independent of the commits it covers.
//!
//! ```text
//! refs/meta/reviews/<review-id> → commit → tree
//! ├── meta            # toml: author, target_branch, state, created
//! ├── description     # markdown
//! └── revisions/
//!     ├── 001         # toml: head_commit, timestamp
//!     └── 002         # toml: head_commit, timestamp
//! ```
//!
//! Each mutation is a new commit on the review's ref. The commit history is
//! the review's audit log.
//!
//! A review does not contain comments or approvals — it prompts them. Comments
//! land on blob OIDs via `git_forge_core::metadata::comments`. Approvals land
//! on patch-ids and OIDs via `git_forge_core::metadata::approvals`. The review
//! is how you discover which commits to look at; the annotations are what you
//! find when you look.

pub mod git2;

/// Ref prefix under which review refs are stored.
pub const REVIEWS_REF_PREFIX: &str = "refs/meta/reviews/";

/// The lifecycle state of a review.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReviewState {
    /// The review is open and awaiting attention.
    Open,
    /// The review's commits were merged into the target branch.
    Merged,
    /// The review was closed without merging.
    Closed,
}

impl ReviewState {
    /// Canonical string representation stored in `meta`.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Open => "open",
            Self::Merged => "merged",
            Self::Closed => "closed",
        }
    }
}

/// Metadata stored in a review's `meta` file.
#[derive(Clone, Debug)]
pub struct ReviewMeta {
    /// Fingerprint of the review author.
    pub author: String,
    /// The branch this review targets (e.g. `refs/heads/main`).
    pub target_branch: String,
    /// Lifecycle state.
    pub state: ReviewState,
    /// RFC 3339 creation timestamp.
    pub created: String,
}

/// A single revision entry in `revisions/`.
#[derive(Clone, Debug)]
pub struct Revision {
    /// Sequential index (e.g. `"001"`, `"002"`).
    pub index: String,
    /// The commit OID at the tip of this revision.
    pub head_commit: ::git2::Oid,
    /// RFC 3339 timestamp when this revision was recorded.
    pub timestamp: String,
}

/// A fully loaded review.
#[derive(Clone, Debug)]
pub struct Review {
    /// Sequential integer ID.
    pub id: u64,
    /// Metadata from the `meta` file.
    pub meta: ReviewMeta,
    /// Markdown description from the `description` file.
    pub description: String,
    /// Ordered list of revisions, oldest first.
    pub revisions: Vec<Revision>,
}

/// Parameters for creating a new review.
#[derive(Clone, Debug)]
pub struct NewReview {
    /// The branch this review targets.
    pub target_branch: String,
    /// Markdown description.
    pub description: String,
    /// The current tip commit of the feature branch.
    pub head_commit: ::git2::Oid,
}

/// Parameters for mutating an existing review.
#[derive(Clone, Debug, Default)]
pub struct ReviewUpdate {
    /// Replace the description when `Some`.
    pub description: Option<String>,
    /// Transition to a new state when `Some`.
    pub state: Option<ReviewState>,
}

/// Operations on review refs under [`REVIEWS_REF_PREFIX`].
pub trait Reviews {
    /// Return the ref name for a specific review ID.
    fn review_ref(id: u64) -> String {
        format!("{REVIEWS_REF_PREFIX}{id}")
    }

    /// Return all reviews, ordered by ID ascending.
    fn list_reviews(&self) -> Result<Vec<Review>, ::git2::Error>;

    /// Return all reviews matching `state`, ordered by ID ascending.
    fn list_reviews_by_state(&self, state: ReviewState) -> Result<Vec<Review>, ::git2::Error>;

    /// Load a single review by ID, returning `None` if the ref does not exist.
    fn find_review(&self, id: u64) -> Result<Option<Review>, ::git2::Error>;

    /// Create a new review, returning the assigned ID.
    fn create_review(&self, review: &NewReview) -> Result<u64, ::git2::Error>;

    /// Apply `update` to the review identified by `id`.
    fn update_review(&self, id: u64, update: &ReviewUpdate) -> Result<(), ::git2::Error>;

    /// Record a new revision for an existing review (the author pushed or
    /// rebased their branch).
    fn add_revision(&self, id: u64, head_commit: ::git2::Oid) -> Result<(), ::git2::Error>;

    /// Compute the commit range `base..tip` for the given revision of a
    /// review, where `base` is the merge base of `head_commit` with
    /// `target_branch`.
    fn revision_range(
        &self,
        review: &Review,
        revision_index: usize,
    ) -> Result<(::git2::Oid, ::git2::Oid), ::git2::Error>;
}
