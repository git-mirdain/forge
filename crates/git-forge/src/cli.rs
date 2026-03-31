//! CLI shape for the `forge` binary.
//!
//! This module contains only clap type definitions — no execution logic.
//! See [`crate::exe`] for the executor.

use std::path::PathBuf;

use clap::{Parser, Subcommand, ValueEnum};

use crate::issue::IssueState;
use crate::review::ReviewState;

/// Local-first Git forge CLI.
#[derive(Parser, Debug)]
#[command(version)]
pub struct Cli {
    /// Output results as JSON.
    #[arg(long)]
    pub json: bool,

    /// Allow operations on a dirty working tree or index.
    #[arg(long)]
    pub allow_dirty: bool,

    /// Subcommand to run.
    #[command(subcommand)]
    pub command: Command,
}

/// Top-level subcommands.
#[derive(Subcommand, Debug)]
pub enum Command {
    /// Manage issues.
    Issue {
        /// Issue subcommand.
        #[command(subcommand)]
        command: IssueCommand,
    },
    /// Manage reviews.
    Review {
        /// Review subcommand.
        #[command(subcommand)]
        command: ReviewCommand,
    },
    /// Manage comments on issues or reviews.
    Comment {
        /// Comment subcommand.
        #[command(subcommand)]
        command: CommentCommand,
    },
    /// Manage provider configuration.
    Config {
        /// Config subcommand.
        #[command(subcommand)]
        command: ConfigCommand,
    },
    /// Manage the forge object index.
    Index {
        /// Index subcommand.
        #[command(subcommand)]
        command: IndexCommand,
    },
}

/// Config subcommands.
#[derive(Subcommand, Debug)]
pub enum ConfigCommand {
    /// Auto-detect provider from git remote URL(s).
    Init {
        /// Remote name to parse (default: origin).
        #[arg(long, short = 'r')]
        remote: Option<String>,
    },
    /// Add a provider config entry.
    Add {
        /// Provider name (e.g. "github").
        provider: String,

        /// Repository owner or organization.
        owner: String,

        /// Repository name.
        repo: String,
    },
    /// List all configured providers.
    List,
    /// Remove a provider config entry.
    Remove {
        /// Provider name.
        provider: String,

        /// Repository owner or organization.
        owner: String,

        /// Repository name.
        repo: String,
    },
    /// Manage contributors.
    Contributor {
        /// Contributor subcommand.
        #[command(subcommand)]
        command: ContributorCommand,
    },
}

/// Contributor subcommands.
#[derive(Subcommand, Debug)]
pub enum ContributorCommand {
    /// Register a contributor.
    Add {
        /// Contributor ID (defaults to git user name).
        #[arg(long)]
        id: Option<String>,

        /// Email addresses (defaults to git user email).
        #[arg(long = "email", short = 'e')]
        emails: Vec<String>,

        /// Display names (defaults to git user name).
        #[arg(long = "name", short = 'n')]
        names: Vec<String>,
    },
    /// List all contributors.
    List,
    /// Remove a contributor.
    Remove {
        /// Contributor ID.
        id: String,
    },
}

/// Filter for comment thread resolved state.
#[derive(ValueEnum, Debug, Clone)]
pub enum CommentStateFilter {
    /// Only unresolved threads.
    Active,
    /// Only resolved threads.
    Resolved,
    /// All threads regardless of state.
    All,
}

/// Comment subcommands.
#[derive(Subcommand, Debug)]
pub enum CommentCommand {
    /// Create a new comment thread anchored to any git object.
    Create {
        /// Anchor spec: raw OID, `HEAD:<path>`, `issue:<id>`, or `review:<id>`.
        #[arg(long)]
        on: String,

        /// Line range within the anchored blob (e.g. `"42-47"` or `"42"`).
        #[arg(long)]
        lines: Option<String>,

        /// Comment body (Markdown).
        body: Option<String>,

        /// Read body from a file.
        #[arg(long, short = 'f')]
        file: Option<PathBuf>,

        /// Prompt for body interactively.
        #[arg(long, short = 'i')]
        interactive: bool,
    },

    /// Reply to an existing comment thread.
    Reply {
        /// Thread UUID.
        thread_id: String,

        /// OID of the comment to reply to.
        #[arg(long = "to")]
        reply_to: String,

        /// Comment body (Markdown).
        body: Option<String>,

        /// Read body from a file.
        #[arg(long, short = 'f')]
        file: Option<PathBuf>,

        /// Prompt for body interactively.
        #[arg(long, short = 'i')]
        interactive: bool,
    },

    /// Resolve a comment thread.
    Resolve {
        /// Thread UUID.
        thread_id: String,

        /// OID of the comment that roots the thread.
        #[arg(long)]
        comment: String,

        /// Optional resolution message.
        message: Option<String>,

        /// Read message from a file.
        #[arg(long, short = 'f')]
        file: Option<PathBuf>,

        /// Prompt for message interactively.
        #[arg(long, short = 'i')]
        interactive: bool,
    },

    /// Edit a comment in a thread.
    Edit {
        /// Thread UUID.
        thread_id: String,

        /// OID of the comment to edit.
        #[arg(long)]
        comment: String,

        /// New body (Markdown).
        #[arg(long)]
        body: String,
    },

    /// List comment threads.
    ///
    /// Use `--on` to scope to one git object, or `--all` to list across the
    /// whole repository.  With `--all`, `--state` filters by resolved state
    /// (default: `active`).
    #[command(group(clap::ArgGroup::new("target").required(true).args(["on", "all"])))]
    List {
        /// Anchor spec: raw OID, `HEAD:<path>`, `issue:<id>`, or `review:<id>`.
        #[arg(long, group = "target")]
        on: Option<String>,

        /// List threads across the entire repository.
        #[arg(long, group = "target")]
        all: bool,

        /// Filter by resolved state (only applies with `--all`).
        #[arg(long, default_value = "active")]
        state: CommentStateFilter,
    },

    /// Show all comments in a thread.
    Show {
        /// Thread UUID.
        thread_id: String,
    },
}

/// Index subcommands.
#[derive(Subcommand, Debug)]
pub enum IndexCommand {
    /// Rebuild the comments-by-object index from all thread refs.
    Rebuild,
}

/// Review subcommands.
#[derive(Subcommand, Debug)]
pub enum ReviewCommand {
    /// Create a new review.
    #[command(group(clap::ArgGroup::new("target").required(true).args(["head", "path"])))]
    New {
        /// Review title.
        #[arg(long)]
        title: Option<String>,

        /// Description (Markdown).
        #[arg(long)]
        body: Option<String>,

        /// Read body from a file.
        #[arg(long, short = 'f')]
        file: Option<PathBuf>,

        /// Head object OID or ref.
        #[arg(long)]
        head: Option<String>,

        /// File or directory path to review (resolved against HEAD).
        #[arg(long, short = 'p')]
        path: Option<PathBuf>,

        /// Base object OID or ref (for commit ranges).
        #[arg(long)]
        base: Option<String>,

        /// Source ref name to track for refreshes.
        #[arg(long = "ref")]
        source_ref: Option<String>,

        /// Prompt for title and description interactively.
        #[arg(long, short = 'i')]
        interactive: bool,
    },

    /// Show a review.
    Show {
        /// Display ID or OID prefix.
        reference: String,
    },

    /// List reviews.
    List {
        /// Filter by state (comma-separated, e.g. `open,closed`).
        #[arg(long)]
        state: Option<String>,
    },

    /// Edit a review.
    Edit {
        /// Display ID or OID prefix.
        reference: String,

        /// New title.
        #[arg(long)]
        title: Option<String>,

        /// New description (Markdown).
        #[arg(long)]
        body: Option<String>,

        /// Read body from a file.
        #[arg(long, short = 'f')]
        file: Option<PathBuf>,

        /// New state.
        #[arg(long)]
        state: Option<ReviewState>,

        /// Prompt for title, description, and state interactively.
        #[arg(long, short = 'i')]
        interactive: bool,
    },

    /// Close a review.
    Close {
        /// Display ID or OID prefix.
        reference: String,
    },

    /// Approve a review.
    Approve {
        /// Display ID or OID prefix.
        reference: String,

        /// Optional approval message.
        message: Option<String>,
    },

    /// Revoke your approval on a review.
    Unapprove {
        /// Display ID or OID prefix.
        reference: String,
    },

    /// List files in the review target.
    Files {
        /// Display ID or OID prefix.
        reference: String,
    },

    /// Show which blobs in a tree lack approved reviews.
    Coverage {
        /// Git revision to check (defaults to HEAD).
        #[arg(default_value = "HEAD")]
        revision: String,
    },

    /// Check out a review into a worktree for commenting.
    Checkout {
        /// Display ID or OID prefix.
        reference: String,

        /// Worktree path (default: ../<repo-name>.review/<reference>).
        path: Option<PathBuf>,
    },

    /// Remove a review worktree created by `checkout`.
    Done {
        /// Display ID or OID prefix (inferred from active worktree if omitted).
        reference: Option<String>,
    },

    /// Retarget a review to a new head, migrating carry-forward comments.
    Retarget {
        /// Display ID or OID prefix.
        reference: String,

        /// New head object OID or ref.
        #[arg(long)]
        head: String,
    },
}

/// Issue subcommands.
#[derive(Subcommand, Debug)]
pub enum IssueCommand {
    /// Create a new issue.
    New {
        /// Issue title (prompted interactively if omitted).
        #[arg(long)]
        title: Option<String>,

        /// Issue body (Markdown).
        #[arg(long)]
        body: Option<String>,

        /// Read body from a file.
        #[arg(long, short = 'f')]
        file: Option<PathBuf>,

        /// Labels to attach.
        #[arg(long = "label", short = 'l')]
        labels: Vec<String>,

        /// Contributor IDs to assign.
        #[arg(long = "assignee", short = 'a')]
        assignees: Vec<String>,

        /// Prompt for all fields interactively.
        #[arg(long, short = 'i')]
        interactive: bool,
    },

    /// Show an issue.
    Show {
        /// Display ID or OID prefix (e.g. `3`, `ab3f`, `GH1`).
        reference: String,
    },

    /// List issues.
    List {
        /// Filter by state (comma-separated, e.g. `open,closed`).
        #[arg(long)]
        state: Option<String>,

        /// Filter by platform sigil (comma-separated, e.g. `GH#,GL#`).
        #[arg(long, short = 'p')]
        platform: Option<String>,

        /// Filter by display ID or OID prefix (comma-separated).
        #[arg(long)]
        id: Option<String>,
    },

    /// Edit an issue.
    Edit {
        /// Display ID or OID prefix.
        reference: String,

        /// New title.
        #[arg(long)]
        title: Option<String>,

        /// New body (Markdown).
        #[arg(long)]
        body: Option<String>,

        /// Read body from a file.
        #[arg(long, short = 'f')]
        file: Option<PathBuf>,

        /// New state.
        #[arg(long)]
        state: Option<IssueState>,

        /// Labels to add.
        #[arg(long = "add-label")]
        add_labels: Vec<String>,

        /// Labels to remove.
        #[arg(long = "remove-label")]
        remove_labels: Vec<String>,

        /// Assignees to add.
        #[arg(long = "add-assignee")]
        add_assignees: Vec<String>,

        /// Assignees to remove.
        #[arg(long = "remove-assignee")]
        remove_assignees: Vec<String>,

        /// Prompt for title, body, and state interactively.
        #[arg(long, short = 'i')]
        interactive: bool,
    },

    /// Close an issue.
    Close {
        /// Display ID or OID prefix.
        reference: String,
    },

    /// Reopen a closed issue.
    Reopen {
        /// Display ID or OID prefix.
        reference: String,
    },
}
