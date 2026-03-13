//! Check result metadata stored under `refs/metadata/checks/`.
//!
//! Check results are metadata on commits, keyed by run ID to support multiple
//! runs and matrix builds:
//!
//! ```text
//! refs/metadata/checks/<commit-oid>/
//!   <run-id>/
//!     meta    # toml: name, state, started, finished, runner_fingerprint, params
//!     log     # blob: raw output
//! ```
//!
//! Every execution is a distinct tree entry — results are never overwritten.
//! The merge gate queries all runs for a required check name and uses the most
//! recent result.

pub mod git2;

/// The ref prefix under which check result metadata is stored (fanout by
/// commit OID).
pub const CHECKS_REF_PREFIX: &str = "refs/metadata/checks/";

/// The outcome of a single check run.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CheckState {
    /// The check is currently executing.
    Running,
    /// The check completed successfully.
    Pass,
    /// The check completed with a failure.
    Fail,
}

impl CheckState {
    /// Canonical string representation stored in `meta`.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Running => "running",
            Self::Pass => "pass",
            Self::Fail => "fail",
        }
    }
}

/// Optional key-value parameters that distinguish matrix build variants.
#[derive(Clone, Debug, Default)]
pub struct RunParams {
    /// Operating system variant (e.g. `"linux"`, `"macos"`).
    pub os: Option<String>,
    /// CPU architecture variant (e.g. `"amd64"`, `"aarch64"`).
    pub arch: Option<String>,
}

/// Metadata stored in a check result's `meta` file.
#[derive(Clone, Debug)]
pub struct CheckResultMeta {
    /// The check name this result is for.
    pub name: String,
    /// The outcome of this run.
    pub state: CheckState,
    /// RFC 3339 timestamp when the run started.
    pub started: String,
    /// RFC 3339 timestamp when the run finished. `None` if still running.
    pub finished: Option<String>,
    /// Fingerprint of the runner that executed and signed this result.
    pub runner_fingerprint: String,
    /// Optional matrix parameters.
    pub params: RunParams,
}

/// A fully loaded check result including its log output.
#[derive(Clone, Debug)]
pub struct CheckResult {
    /// The commit OID this result is attached to.
    pub commit_oid: ::git2::Oid,
    /// The run ID (timestamp + fingerprint or short random ID).
    pub run_id: String,
    /// Metadata for this run.
    pub meta: CheckResultMeta,
    /// Raw log output from the run.
    pub log: Vec<u8>,
}

/// Parameters for recording a new check result.
#[derive(Clone, Debug)]
pub struct NewCheckResult {
    /// The commit OID to attach the result to.
    pub commit_oid: ::git2::Oid,
    /// The check name.
    pub name: String,
    /// The outcome.
    pub state: CheckState,
    /// RFC 3339 start timestamp.
    pub started: String,
    /// RFC 3339 finish timestamp. `None` when recording a `Running` entry.
    pub finished: Option<String>,
    /// Fingerprint of the signing runner.
    pub runner_fingerprint: String,
    /// Optional matrix parameters.
    pub params: RunParams,
    /// Raw log output.
    pub log: Vec<u8>,
}

/// Operations on check result metadata stored under [`CHECKS_REF_PREFIX`].
pub trait CheckResults {
    /// Return all check results recorded for `commit_oid`, across all run IDs.
    fn check_results_for(&self, commit_oid: ::git2::Oid)
    -> Result<Vec<CheckResult>, ::git2::Error>;

    /// Return only the most recent result for each distinct check name on
    /// `commit_oid`. This is the view the merge gate uses.
    fn latest_check_results(
        &self,
        commit_oid: ::git2::Oid,
    ) -> Result<Vec<CheckResult>, ::git2::Error>;

    /// Return the most recent result for the check named `name` on
    /// `commit_oid`, or `None` if no result has been recorded.
    fn latest_check_result(
        &self,
        commit_oid: ::git2::Oid,
        name: &str,
    ) -> Result<Option<CheckResult>, ::git2::Error>;

    /// Record a new check result, assigning a fresh run ID.
    ///
    /// Returns the assigned run ID. Never overwrites an existing entry — every
    /// execution is a distinct tree entry.
    fn record_check_result(&self, result: &NewCheckResult) -> Result<String, ::git2::Error>;

    /// Return `true` if every check name in `required` has at least one
    /// `Pass` result as its most recent run on `commit_oid`.
    fn required_checks_pass(
        &self,
        commit_oid: ::git2::Oid,
        required: &[&str],
    ) -> Result<bool, ::git2::Error>;
}
