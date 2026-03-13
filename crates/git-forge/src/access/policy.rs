//! Role definitions and branch policy from `.forge/policy.toml`.
//!
//! Policy is a TOML file committed to the repository. Roles map to permission
//! sets; path-scoped permissions restrict which files a role may modify.
//!
//! ```text
//! # .forge/policy.toml
//!
//! [roles.admin]
//! push = ["refs/heads/*"]
//! approve = true
//! manage_issues = true
//! modify_contributors = true
//! modify_policy = true
//!
//! [branches.main]
//! merge_strategy = "squash"
//! approval_check = "range"
//! min_approvals = 1
//! exclude_author = true
//! block_unresolved_comments = true
//! require_checks = ["build", "test"]
//! ```
//!
//! ## Self-protecting policy
//!
//! Changing `policy.toml` requires satisfying the rules currently in effect.
//! The pre-receive hook evaluates the policy at the incoming commits, not at
//! `HEAD` — you cannot weaken policy without first meeting it.

pub mod git2;

/// The working-tree path of the policy file.
pub const POLICY_PATH: &str = ".forge/policy.toml";

/// The merge strategy applied when a review is merged into a protected branch.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MergeStrategy {
    /// Merge all commits as-is, preserving history.
    Merge,
    /// Rebase the commits onto the target branch tip.
    Rebase,
    /// Squash all commits into a single commit.
    Squash,
}

impl MergeStrategy {
    /// Canonical string representation stored in `policy.toml`.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Merge => "merge",
            Self::Rebase => "rebase",
            Self::Squash => "squash",
        }
    }
}

/// Which level of approval granularity the merge gate checks.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ApprovalCheck {
    /// Each individual commit's patch-id must be approved.
    PerPatch,
    /// The range patch-id covering the entire change set must be approved.
    Range,
}

impl ApprovalCheck {
    /// Canonical string representation stored in `policy.toml`.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::PerPatch => "per_patch",
            Self::Range => "range",
        }
    }
}

/// The object type checked for state approval.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StateApprovalType {
    /// Require approval of the blob OID for each matching path.
    Blob,
    /// Require approval of the tree OID for each matching directory.
    Tree,
}

impl StateApprovalType {
    /// Canonical string representation stored in `policy.toml`.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Blob => "blob",
            Self::Tree => "tree",
        }
    }
}

/// State-approval requirement for a set of paths.
#[derive(Clone, Debug)]
pub struct StateApprovalPolicy {
    /// Glob patterns selecting the paths subject to state approval.
    pub paths: Vec<String>,
    /// Whether blob or tree OID approval is required.
    pub approval_type: StateApprovalType,
    /// Fingerprints or group names permitted to provide the approval.
    pub approvers: Vec<String>,
}

/// Policy governing pushes to a single protected branch.
#[derive(Clone, Debug)]
pub struct BranchPolicy {
    /// The ref pattern this policy applies to (e.g. `"refs/heads/main"`).
    pub branch_ref: String,
    /// How commits are integrated when a review is merged.
    pub merge_strategy: MergeStrategy,
    /// Which approval granularity is checked at the merge gate.
    pub approval_check: ApprovalCheck,
    /// Minimum number of distinct approvals required.
    pub min_approvals: usize,
    /// When `true`, the review author's own approvals are not counted.
    pub exclude_author: bool,
    /// When `true`, the merge gate rejects if any comment on affected blobs is
    /// unresolved.
    pub block_unresolved_comments: bool,
    /// Optional state-approval requirements for specific path patterns.
    pub state_approval: Option<StateApprovalPolicy>,
    /// Check names that must have passed on the merge commit.
    pub require_checks: Vec<String>,
}

/// Permissions granted to a named role.
#[derive(Clone, Debug, Default)]
pub struct RolePermissions {
    /// Ref patterns this role may push to.
    pub push: Vec<String>,
    /// When `true`, this role may record approvals that count toward policy.
    pub approve: bool,
    /// When `true`, this role may create, update, and close issues.
    pub manage_issues: bool,
    /// When `true`, this role may add and remove entries from
    /// `refs/meta/contributors`.
    pub modify_contributors: bool,
    /// When `true`, this role may modify `policy.toml`.
    pub modify_policy: bool,
    /// Optional path-scoped push restriction. When non-empty, pushes are
    /// further restricted to commits that only touch these glob patterns.
    pub paths: Vec<String>,
}

/// The fully parsed contents of `.forge/policy.toml`.
#[derive(Clone, Debug, Default)]
pub struct Policy {
    /// Per-role permission definitions, keyed by role name.
    pub roles: std::collections::HashMap<String, RolePermissions>,
    /// Per-branch merge-gate policies, keyed by branch ref pattern.
    pub branches: std::collections::HashMap<String, BranchPolicy>,
}

/// Operations for reading and evaluating repository access policy.
pub trait AccessPolicy {
    /// Parse and return the policy at `HEAD`'s `.forge/policy.toml`.
    ///
    /// Returns a default (permissive) [`Policy`] when the file does not yet
    /// exist.
    fn load_policy(&self) -> Result<Policy, ::git2::Error>;

    /// Parse and return the policy at the tree of `commit_oid`.
    ///
    /// The pre-receive hook uses this to evaluate policy as it exists in the
    /// commits being pushed — specifically to enforce self-protecting policy.
    fn load_policy_at(&self, commit_oid: ::git2::Oid) -> Result<Policy, ::git2::Error>;

    /// Return the [`BranchPolicy`] that governs `branch_ref`, or `None` if no
    /// policy has been configured for that branch.
    fn branch_policy(&self, branch_ref: &str) -> Result<Option<BranchPolicy>, ::git2::Error>;

    /// Return the [`RolePermissions`] for `role_name`, or `None` if the role
    /// is not defined in policy.
    fn role_permissions(&self, role_name: &str) -> Result<Option<RolePermissions>, ::git2::Error>;

    /// Return `true` if `fingerprint` is permitted to push to `target_ref`,
    /// optionally restricted to the set of paths in `changed_paths`.
    ///
    /// Combines the contributor's roles with the policy's push rules and any
    /// path-scoped restrictions.
    fn check_push_permission(
        &self,
        fingerprint: &str,
        target_ref: &str,
        changed_paths: &[&str],
    ) -> Result<bool, ::git2::Error>;
}
