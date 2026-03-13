//! Merge gate: evaluates whether a push satisfies all policy requirements.
//!
//! The pre-receive hook enforces policy on pushes to protected branches. For
//! each push it:
//!
//! 1. Verifies the push is signed by a known contributor.
//! 2. Checks the contributor's role permits pushing to this ref.
//! 3. Checks path-scoped permissions via `git diff --name-only`.
//! 4. Checks approvals per the branch's approval policy setting.
//! 5. Verifies required CI checks passed on each commit.
//! 6. Optionally rejects if any unresolved comments exist on affected blobs.
//!
//! The gate returns a complete list of all violations so the pusher receives a
//! full picture of what is missing, not just the first failure.

pub mod git2;

/// The result of a merge gate evaluation.
#[derive(Clone, Debug)]
pub enum GateOutcome {
    /// The push satisfies all policy requirements and is accepted.
    Accept,
    /// The push violates one or more requirements.
    Reject(Vec<GateViolation>),
}

/// A single policy violation that caused the gate to reject a push.
#[derive(Clone, Debug)]
pub enum GateViolation {
    /// The pusher's fingerprint is not in `refs/meta/contributors`.
    UnknownContributor {
        /// The unrecognised key fingerprint.
        fingerprint: String,
    },
    /// The contributor's role does not permit pushing to this ref.
    PushNotPermitted {
        /// The pusher's fingerprint.
        fingerprint: String,
        /// The ref the push was targeting.
        target_ref: String,
    },
    /// A path-scoped permission check failed.
    PathNotPermitted {
        /// The file path that exceeded the role's path scope.
        path: String,
        /// The role whose path restriction was violated.
        role: String,
    },
    /// Fewer distinct approvals were found than policy requires.
    InsufficientApprovals {
        /// The patch-id or OID that lacked sufficient approvals.
        object_id: String,
        /// Number of distinct approvals found.
        found: usize,
        /// Minimum required by policy.
        required: usize,
    },
    /// A required CI check has not passed on the commit.
    CheckNotPassed {
        /// The commit whose check result was missing or failed.
        commit_oid: ::git2::Oid,
        /// The name of the required check (e.g. `"build"`, `"test"`).
        check_name: String,
    },
    /// One or more comments on affected blobs are still unresolved.
    UnresolvedComments {
        /// The blob OID that has unresolved comments.
        blob_oid: ::git2::Oid,
        /// IDs of the unresolved comments on that blob.
        comment_ids: Vec<String>,
    },
    /// The policy itself disallows the modification (self-protecting policy).
    PolicyViolation {
        /// Human-readable description of the policy rule that was violated.
        detail: String,
    },
}

/// A push event presented to the merge gate for evaluation.
#[derive(Clone, Debug)]
pub struct PushEvent {
    /// Fingerprint of the pusher (from the signed push certificate).
    pub pusher_fingerprint: String,
    /// The ref being updated.
    pub target_ref: String,
    /// The current tip of the ref before the push (`None` for a new ref).
    pub old_oid: Option<::git2::Oid>,
    /// The proposed new tip of the ref.
    pub new_oid: ::git2::Oid,
}

/// Evaluates whether a push satisfies all policy requirements.
///
/// This is the programmatic interface used by the pre-receive hook. It does
/// not perform the push itself — that is the transport's responsibility.
pub trait MergeGate {
    /// Evaluate `event` against the current policy and return the outcome.
    ///
    /// A [`GateOutcome::Reject`] result carries a list of all violations found
    /// so the pusher receives a complete picture of what is missing.
    fn evaluate_push(&self, event: &PushEvent) -> Result<GateOutcome, ::git2::Error>;
}
