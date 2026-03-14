//! Issue refs under `refs/meta/issues/`.
//!
//! An issue is a standalone ref with its own lifecycle:
//!
//! ```text
//! refs/meta/issues/<issue-id> → commit → tree
//! ├── meta            # toml: author, title, state, labels, assignees, created
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

pub mod git2;

/// Ref prefix under which issue refs are stored.
pub const ISSUES_REF_PREFIX: &str = "refs/meta/issues/";

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

/// Metadata stored in an issue's `meta` file.
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
    /// Fingerprints of assigned contributors.
    pub assignees: Vec<String>,
    /// RFC 3339 creation timestamp.
    pub created: String,
}

/// A fully loaded issue.
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

/// Parameters for creating a new issue.
#[derive(Clone, Debug)]
pub struct NewIssue {
    /// Single-line title.
    pub title: String,
    /// Markdown body.
    pub body: String,
    /// Optional initial labels.
    pub labels: Vec<String>,
    /// Optional initial assignees (fingerprints).
    pub assignees: Vec<String>,
}

/// Parameters for mutating an existing issue.
#[derive(Clone, Debug, Default)]
pub struct IssueUpdate {
    /// Replace the title when `Some`.
    pub title: Option<String>,
    /// Replace the body when `Some`.
    pub body: Option<String>,
    /// Replace the full label list when `Some`.
    pub labels: Option<Vec<String>>,
    /// Replace the full assignee list when `Some`.
    pub assignees: Option<Vec<String>>,
    /// Transition to a new state when `Some`.
    pub state: Option<IssueState>,
}

/// Operations on issue refs under [`ISSUES_REF_PREFIX`].
pub trait Issues {
    /// Return the ref name for a specific issue ID.
    fn issue_ref(id: u64) -> String {
        format!("{ISSUES_REF_PREFIX}{id}")
    }

    /// Return all issues, ordered by ID ascending.
    fn list_issues(&self) -> Result<Vec<Issue>, ::git2::Error>;

    /// Return all issues matching `state`, ordered by ID ascending.
    fn list_issues_by_state(&self, state: IssueState) -> Result<Vec<Issue>, ::git2::Error>;

    /// Load a single issue by ID, returning `None` if the ref does not exist.
    fn find_issue(&self, id: u64) -> Result<Option<Issue>, ::git2::Error>;

    /// Create a new issue, returning the assigned ID.
    fn create_issue(&self, issue: &NewIssue) -> Result<u64, ::git2::Error>;

    /// Apply `update` to the issue identified by `id`.
    fn update_issue(&self, id: u64, update: &IssueUpdate) -> Result<(), ::git2::Error>;

    /// Add a conversation comment to an issue (not a code comment).
    fn add_issue_comment(&self, id: u64, author: &str, body: &str) -> Result<(), ::git2::Error>;
}
