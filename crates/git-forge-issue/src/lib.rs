//! Issue refs under `refs/meta/issues/`.
//!
//! An issue is a standalone ref with its own lifecycle:
//!
//! ```text
//! refs/issue/<issue-id> → commit → tree
//! ├── author          # plain text: fingerprint
//! ├── title           # plain text: single-line title
//! ├── state           # plain text: "open" or "closed"
//! ├── labels/         # dir: empty blobs whose names are the labels
//! ├── body            # markdown
//! └── comments/
//!     ├── 001-<ts>-<fingerprint>      # markdown
//!     └── 002-<ts>-<fingerprint>
//! ```
//!
//! Each mutation is a new commit on the issue's ref. The commit history is the
//! issue's audit log.
//!
//! Issue comments are conversation within the issue. They are not the same as
//! code comments — those live in `git_forge_core::metadata::comments`.

#[cfg(test)]
mod tests;

pub mod cli;
pub mod exe;
pub mod git2;

/// Ref prefix under which issue refs are stored.
pub const ISSUES_REF_PREFIX: &str = "refs/issue/";

/// Options for issue operations, allowing customization of the ref prefix.
#[derive(Clone, Debug)]
pub struct IssueOpts {
    /// Ref prefix under which issue refs are stored. Defaults to [`ISSUES_REF_PREFIX`].
    pub ref_prefix: String,
}

impl Default for IssueOpts {
    fn default() -> Self {
        Self {
            ref_prefix: ISSUES_REF_PREFIX.to_string(),
        }
    }
}

/// The lifecycle state of an issue.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IssueState {
    /// The issue is active and unresolved.
    Open,
    /// The issue has been resolved or won't be fixed.
    Closed,
}

impl IssueState {
    /// Canonical string representation stored in `meta`.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Open => "open",
            Self::Closed => "closed",
        }
    }
}

/// Metadata for an issue.
#[derive(Clone, Debug)]
pub struct IssueMeta {
    /// Fingerprint of the issue author.
    pub author: String,
    /// Single-line title.
    pub title: String,
    /// Lifecycle state.
    pub state: IssueState,
    /// Free-form label strings.
    pub labels: Vec<String>,
}

/// Represents an issue that could exist under e.g. `refs/issue`.
#[derive(Clone, Debug)]
pub struct Issue {
    /// Sequential integer ID.
    pub id: u64,
    /// Metadata from the `meta` file.
    pub meta: IssueMeta,
    /// Markdown body from the `body` file.
    pub body: String,
    /// Issue-scoped comments. Each entry is `(filename, markdown_body)`.
    pub comments: Vec<(String, String)>,
}

/// Operations on issue refs under [`ISSUES_REF_PREFIX`].
pub trait Issues {
    /// Return the ref name for a specific issue ID.
    #[must_use]
    fn issue_ref(id: u64) -> String {
        format!("{ISSUES_REF_PREFIX}{id}")
    }

    /// Return all issues, ordered by ID ascending.
    ///
    /// # Errors
    ///
    /// Returns `git2::Error` if the underlying repository operation fails.
    fn list_issues(&self, opts: Option<&IssueOpts>) -> Result<Vec<Issue>, ::git2::Error>;

    /// Return all issues matching `state`, ordered by ID ascending.
    ///
    /// # Errors
    ///
    /// Returns `git2::Error` if the underlying repository operation fails.
    fn list_issues_by_state(
        &self,
        state: IssueState,
        opts: Option<&IssueOpts>,
    ) -> Result<Vec<Issue>, ::git2::Error>;

    /// Load a single issue by ID, returning `None` if the ref does not exist.
    ///
    /// # Errors
    ///
    /// Returns `git2::Error` if the underlying repository operation fails.
    fn find_issue(&self, id: u64, opts: Option<&IssueOpts>)
        -> Result<Option<Issue>, ::git2::Error>;

    /// Create a new issue, returning the assigned ID.
    ///
    /// # Errors
    ///
    /// Returns `git2::Error` if the underlying repository operation fails.
    fn create_issue(
        &self,
        title: &str,
        body: &str,
        labels: &[String],
        assignees: &[String],
        opts: Option<&IssueOpts>,
    ) -> Result<u64, ::git2::Error>;

    /// Apply `update` to the issue identified by `id`.
    ///
    /// # Errors
    ///
    /// Returns `git2::Error` if the underlying repository operation fails.
    #[allow(clippy::too_many_arguments)]
    fn update_issue(
        &self,
        id: u64,
        title: Option<&str>,
        body: Option<&str>,
        labels: Option<&[String]>,
        assignees: Option<&[String]>,
        state: Option<IssueState>,
        opts: Option<&IssueOpts>,
    ) -> Result<(), ::git2::Error>;

    /// Add a conversation comment to an issue (not a code comment).
    ///
    /// # Errors
    ///
    /// Returns `git2::Error` if the underlying repository operation fails.
    fn add_issue_comment(
        &self,
        id: u64,
        author: &str,
        body: &str,
        opts: Option<&IssueOpts>,
    ) -> Result<(), ::git2::Error>;
}
