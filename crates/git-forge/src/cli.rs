//! CLI shape for the `forge` binary.
//!
//! This module contains only clap type definitions — no execution logic.
//! See [`crate::exe`] for the executor.

use std::path::PathBuf;

use clap::{Parser, Subcommand};

use crate::issue::IssueState;

/// Local-first Git forge CLI.
#[derive(Parser, Debug)]
#[command(version)]
pub struct Cli {
    /// Output results as JSON.
    #[arg(long)]
    pub json: bool,

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
    /// Manage comments on issues.
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
}

/// Comment subcommands.
#[derive(Subcommand, Debug)]
pub enum CommentCommand {
    /// Add a top-level comment to an issue.
    Add {
        /// Issue display ID or OID prefix.
        #[arg(long)]
        issue: String,

        /// Comment body (Markdown).
        body: Option<String>,

        /// Read body from a file.
        #[arg(long, short = 'f')]
        file: Option<PathBuf>,
    },

    /// Reply to an existing comment.
    Reply {
        /// Issue display ID or OID prefix.
        #[arg(long)]
        issue: String,

        /// OID of the comment to reply to.
        #[arg(long = "to")]
        reply_to: String,

        /// Comment body (Markdown).
        body: Option<String>,

        /// Read body from a file.
        #[arg(long, short = 'f')]
        file: Option<PathBuf>,
    },

    /// Resolve a comment thread.
    Resolve {
        /// Issue display ID or OID prefix.
        #[arg(long)]
        issue: String,

        /// OID of the comment that starts the thread.
        #[arg(long = "thread")]
        thread: String,

        /// Optional resolution message.
        message: Option<String>,

        /// Read message from a file.
        #[arg(long, short = 'f')]
        file: Option<PathBuf>,
    },

    /// List comments on an issue.
    List {
        /// Issue display ID or OID prefix.
        #[arg(long)]
        issue: String,
    },
}

/// Issue subcommands.
#[derive(Subcommand, Debug)]
pub enum IssueCommand {
    /// Create a new issue.
    New {
        /// Issue title (prompted interactively if omitted).
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
