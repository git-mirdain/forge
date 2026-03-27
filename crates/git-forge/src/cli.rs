//! CLI shape for the `forge` binary.
//!
//! This module contains only clap type definitions — no execution logic.
//! See [`crate::exe`] for the executor.

use clap::{Parser, Subcommand};

use crate::issue::IssueState;

/// Local-first Git forge CLI.
#[derive(Parser)]
#[command(name = "forge", author, version)]
pub struct Cli {
    /// Output results as JSON.
    #[arg(long, global = true)]
    pub json: bool,

    /// Subcommand to run.
    #[command(subcommand)]
    pub command: Command,
}

/// Top-level subcommands.
#[derive(Subcommand)]
pub enum Command {
    /// Manage issues.
    Issue {
        /// Issue subcommand.
        #[command(subcommand)]
        command: IssueCommand,
    },
}

/// Issue subcommands.
#[derive(Subcommand)]
pub enum IssueCommand {
    /// Create a new issue.
    New {
        /// Issue title.
        title: String,

        /// Issue body (Markdown).
        #[arg(long)]
        body: Option<String>,

        /// Labels to attach.
        #[arg(long = "label", short = 'l')]
        labels: Vec<String>,

        /// Contributor IDs to assign.
        #[arg(long = "assignee", short = 'a')]
        assignees: Vec<String>,
    },

    /// Show an issue.
    Show {
        /// Display ID or OID prefix (e.g. `3`, `ab3f`, `GH1`).
        reference: String,
    },

    /// List issues.
    List {
        /// Filter by state.
        #[arg(long)]
        state: Option<IssueState>,
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
