//! CLI shape for the `forge` binary.
//!
//! This module contains only clap type definitions — no execution logic.
//! See [`crate::exe`] for the executor.

use std::path::PathBuf;

use clap::{Parser, Subcommand};

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

/// Comment subcommands.
#[derive(Subcommand, Debug)]
pub enum CommentCommand {
    /// Add a top-level comment to an issue, review, or object.
    #[command(group(clap::ArgGroup::new("entity").args(["issue", "review", "object"])))]
    Add {
        /// Issue display ID or OID prefix.
        #[arg(long)]
        issue: Option<String>,

        /// Review display ID or OID prefix.
        #[arg(long)]
        review: Option<String>,

        /// Git object OID or revspec to comment on.
        #[arg(long)]
        object: Option<String>,

        /// Anchor the comment to a git object (blob, commit, or tree OID).
        #[arg(long, conflicts_with_all = ["anchor_start", "anchor_end"])]
        anchor: Option<String>,

        /// File path within the anchored object.
        #[arg(long, requires = "anchor")]
        anchor_path: Option<String>,

        /// Line range within the anchored object (e.g. "10-20").
        #[arg(long, requires = "anchor")]
        range: Option<String>,

        /// Start OID for a commit-range anchor.
        #[arg(long, requires = "anchor_end", conflicts_with = "anchor")]
        anchor_start: Option<String>,

        /// End OID for a commit-range anchor.
        #[arg(long, requires = "anchor_start", conflicts_with = "anchor")]
        anchor_end: Option<String>,

        /// Comment body (Markdown).
        body: Option<String>,

        /// Read body from a file.
        #[arg(long, short = 'f')]
        file: Option<PathBuf>,

        /// Prompt for body interactively.
        #[arg(long, short = 'i')]
        interactive: bool,
    },

    /// Reply to an existing comment.
    #[command(group(clap::ArgGroup::new("entity").args(["issue", "review", "object"])))]
    Reply {
        /// Issue display ID or OID prefix.
        #[arg(long)]
        issue: Option<String>,

        /// Review display ID or OID prefix.
        #[arg(long)]
        review: Option<String>,

        /// Git object OID to reply on.
        #[arg(long)]
        object: Option<String>,

        /// OID of the comment to reply to.
        #[arg(long = "to")]
        reply_to: String,

        /// Anchor the reply to a git object (blob, commit, or tree OID).
        #[arg(long, conflicts_with_all = ["anchor_start", "anchor_end"])]
        anchor: Option<String>,

        /// File path within the anchored object.
        #[arg(long, requires = "anchor")]
        anchor_path: Option<String>,

        /// Line range within the anchored object (e.g. "10-20").
        #[arg(long, requires = "anchor")]
        range: Option<String>,

        /// Start OID for a commit-range anchor.
        #[arg(long, requires = "anchor_end", conflicts_with = "anchor")]
        anchor_start: Option<String>,

        /// End OID for a commit-range anchor.
        #[arg(long, requires = "anchor_start", conflicts_with = "anchor")]
        anchor_end: Option<String>,

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
    #[command(group(clap::ArgGroup::new("entity").args(["issue", "review", "object"])))]
    Resolve {
        /// Issue display ID or OID prefix.
        #[arg(long)]
        issue: Option<String>,

        /// Review display ID or OID prefix.
        #[arg(long)]
        review: Option<String>,

        /// Git object OID.
        #[arg(long)]
        object: Option<String>,

        /// OID of the comment that starts the thread.
        #[arg(long = "thread")]
        thread: String,

        /// Optional resolution message.
        message: Option<String>,

        /// Read message from a file.
        #[arg(long, short = 'f')]
        file: Option<PathBuf>,

        /// Prompt for message interactively.
        #[arg(long, short = 'i')]
        interactive: bool,
    },

    /// List comments on an issue, review, or object.
    #[command(group(clap::ArgGroup::new("entity").args(["issue", "review", "object"])))]
    List {
        /// Issue display ID or OID prefix.
        #[arg(long)]
        issue: Option<String>,

        /// Review display ID or OID prefix.
        #[arg(long)]
        review: Option<String>,

        /// Git object OID.
        #[arg(long)]
        object: Option<String>,

        /// Filter comments by anchor path.
        #[arg(long)]
        path: Option<String>,
    },
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
