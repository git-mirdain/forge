//! Approvals on Git objects: blobs, trees, commits, and commit ranges.
//!
//! Approvals attest to correctness at four granularities:
//!
//! | Level  | Object         | Meaning                          |
//! |--------|----------------|----------------------------------|
//! | Blob   | blob OID       | "This file is correct"           |
//! | Tree   | tree OID       | "This subtree is correct"        |
//! | Patch  | patch-id       | "This change is correct"         |
//! | Range  | range patch-id | "This overall change is correct" |
//!
//! Approvals are stored as `git-metadata` entries:
//!
//! ```text
//! refs/metadata/approvals   (fanout by patch-id or oid)
//!   <id>/
//!     <fingerprint>    # toml: timestamp, type, path (blob/tree only), message
//! ```
//!
//! Using patch-id rather than commit OID means approvals survive rebases
//! automatically — the same change before and after rebase produces the same
//! patch-id.

/// The ref under which all approval metadata is stored (fanout by patch-id or
/// object OID).
pub const APPROVALS_REF: &str = "refs/metadata/approvals";

/// The granularity at which an approval is recorded.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ApprovalKind {
    /// Attests to a specific blob OID ("this file is correct").
    Blob,
    /// Attests to a specific tree OID ("this subtree is correct").
    Tree,
    /// Attests to a single commit's patch-id ("this change is correct").
    Patch,
    /// Attests to a range patch-id ("this overall change is correct").
    Range,
}

/// A recorded approval entry.
#[derive(Clone, Debug)]
pub struct Approval {
    /// The object being approved (patch-id hex, blob OID hex, or tree OID hex).
    pub object_id: String,
    /// Fingerprint of the approver.
    pub approver: String,
    /// RFC 3339 timestamp.
    pub timestamp: String,
    /// Granularity of this approval.
    pub kind: ApprovalKind,
    /// For blob/tree approvals: the path that was approved.
    pub path: Option<String>,
    /// Optional free-form message from the approver.
    pub message: Option<String>,
}

/// Parameters for recording a new approval.
#[derive(Clone, Debug)]
pub struct NewApproval {
    /// The object being approved (patch-id hex, blob OID hex, or tree OID hex).
    pub object_id: String,
    /// Granularity of the approval.
    pub kind: ApprovalKind,
    /// For blob/tree approvals: the path that was approved.
    pub path: Option<String>,
    /// Optional free-form message.
    pub message: Option<String>,
}

/// Operations on approvals stored under [`APPROVALS_REF`].
pub trait Approvals {
    /// Return all approvals recorded for `object_id` (patch-id or OID hex).
    fn approvals_for(&self, object_id: &str) -> Result<Vec<Approval>, git2::Error>;

    /// Return the approval by `approver_fingerprint` for `object_id`, or
    /// `None` if the approver has not yet approved.
    fn find_approval(
        &self,
        object_id: &str,
        approver_fingerprint: &str,
    ) -> Result<Option<Approval>, git2::Error>;

    /// Record a new approval.
    fn add_approval(&self, approval: &NewApproval) -> Result<(), git2::Error>;

    /// Bulk-approve every patch-id in `patch_ids` plus a single range approval
    /// for `range_patch_id`. This is what `git forge review approve` calls
    /// under the hood.
    fn bulk_approve_range(
        &self,
        patch_ids: &[String],
        range_patch_id: &str,
        message: Option<&str>,
    ) -> Result<(), git2::Error>;

    /// Return `true` if `object_id` has at least `min_approvals` distinct
    /// approvals, excluding `exclude` when `Some`.
    fn approval_count_satisfies(
        &self,
        object_id: &str,
        min_approvals: usize,
        exclude: Option<&str>,
    ) -> Result<bool, git2::Error>;
}
