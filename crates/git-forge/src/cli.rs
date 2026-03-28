//! CLI shape for the `forge` binary.
//!
//! This module contains only figue type definitions — no execution logic.
//! See [`crate::exe`] for the executor.

use facet::Facet;
use figue::{self as args, FigueBuiltins};

use crate::issue::IssueState;

/// Local-first Git forge CLI.
#[derive(Facet, Debug)]
pub struct Cli {
    /// Output results as JSON.
    #[facet(args::named)]
    pub json: bool,

    /// Subcommand to run.
    #[facet(args::subcommand)]
    pub command: Command,

    /// Built-in flags (--help, --version, --completions).
    #[facet(flatten)]
    pub builtins: FigueBuiltins,
}

/// Top-level subcommands.
#[derive(Facet, Debug)]
#[repr(u8)]
pub enum Command {
    /// Manage issues.
    Issue {
        /// Issue subcommand.
        #[facet(args::subcommand)]
        command: IssueCommand,
    },
    /// Manage provider configuration.
    Config {
        /// Config subcommand.
        #[facet(args::subcommand)]
        command: ConfigCommand,
    },
}

/// Config subcommands.
#[derive(Facet, Debug)]
#[repr(u8)]
pub enum ConfigCommand {
    /// Auto-detect provider from git remote URL(s).
    Init {
        /// Remote name to parse (default: origin).
        #[facet(args::named, args::short = 'r')]
        remote: Option<String>,
    },
    /// Add a provider config entry.
    Add {
        /// Provider name (e.g. "github").
        #[facet(args::positional)]
        provider: String,

        /// Repository owner or organization.
        #[facet(args::positional)]
        owner: String,

        /// Repository name.
        #[facet(args::positional)]
        repo: String,

        /// Sigil prefix for cross-references.
        #[facet(args::named)]
        sigil: Option<String>,
    },
    /// List all configured providers.
    List,
    /// Remove a provider config entry.
    Remove {
        /// Provider name.
        #[facet(args::positional)]
        provider: String,

        /// Repository owner or organization.
        #[facet(args::positional)]
        owner: String,

        /// Repository name.
        #[facet(args::positional)]
        repo: String,
    },
}

/// Issue subcommands.
#[derive(Facet, Debug)]
#[repr(u8)]
pub enum IssueCommand {
    /// Create a new issue.
    New {
        /// Issue title (prompted interactively if omitted).
        #[facet(args::positional)]
        title: Option<String>,

        /// Issue body (Markdown).
        #[facet(args::named)]
        body: Option<String>,

        /// Labels to attach.
        #[facet(args::named, args::short = 'l', rename = "label")]
        labels: Vec<String>,

        /// Contributor IDs to assign.
        #[facet(args::named, args::short = 'a', rename = "assignee")]
        assignees: Vec<String>,

        /// Prompt for all fields interactively.
        #[facet(args::named, args::short = 'i')]
        interactive: bool,
    },

    /// Show an issue.
    Show {
        /// Display ID or OID prefix (e.g. `3`, `ab3f`, `GH1`).
        #[facet(args::positional)]
        reference: String,
    },

    /// List issues.
    List {
        /// Filter by state.
        #[facet(args::named)]
        state: Option<IssueState>,
    },

    /// Edit an issue.
    Edit {
        /// Display ID or OID prefix.
        #[facet(args::positional)]
        reference: String,

        /// New title.
        #[facet(args::named)]
        title: Option<String>,

        /// New body (Markdown).
        #[facet(args::named)]
        body: Option<String>,

        /// New state.
        #[facet(args::named)]
        state: Option<IssueState>,

        /// Labels to add.
        #[facet(args::named, rename = "add-label")]
        add_labels: Vec<String>,

        /// Labels to remove.
        #[facet(args::named, rename = "remove-label")]
        remove_labels: Vec<String>,

        /// Assignees to add.
        #[facet(args::named, rename = "add-assignee")]
        add_assignees: Vec<String>,

        /// Assignees to remove.
        #[facet(args::named, rename = "remove-assignee")]
        remove_assignees: Vec<String>,

        /// Prompt for title, body, and state interactively.
        #[facet(args::named, args::short = 'i')]
        interactive: bool,
    },

    /// Close an issue.
    Close {
        /// Display ID or OID prefix.
        #[facet(args::positional)]
        reference: String,
    },

    /// Reopen a closed issue.
    Reopen {
        /// Display ID or OID prefix.
        #[facet(args::positional)]
        reference: String,
    },
}
