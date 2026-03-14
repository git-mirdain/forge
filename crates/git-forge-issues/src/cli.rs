//! CLI definitions for `git forge issue`.

use clap::Subcommand;

/// Subcommands for `git forge issue`.
#[derive(Subcommand, Clone)]
pub enum IssueCommand {
    /// Open a new issue.
    New {
        /// Issue title.
        title: String,

        /// Issue body (markdown). Reads from stdin if omitted.
        #[arg(short, long)]
        body: Option<String>,

        /// Labels to apply.
        #[arg(short, long)]
        label: Vec<String>,

        /// Assignees (fingerprints or names).
        #[arg(short, long)]
        assignee: Vec<String>,
    },

    /// Edit an existing issue.
    Edit {
        /// Issue ID.
        id: u64,

        /// New title.
        #[arg(short, long)]
        title: Option<String>,

        /// New body.
        #[arg(short, long)]
        body: Option<String>,

        /// Replace labels with this set.
        #[arg(short, long)]
        label: Vec<String>,

        /// Replace assignees with this set.
        #[arg(short, long)]
        assignee: Vec<String>,

        /// Set state to open or closed.
        #[arg(long, value_enum)]
        state: Option<StateArg>,
    },

    /// List issues.
    List {
        /// Filter by state.
        #[arg(long, value_enum, default_value_t = StateArg::Open)]
        state: StateArg,
    },

    /// Show the status of an issue.
    Status {
        /// Issue ID.
        id: u64,
    },

    /// Show details of an issue.
    Show {
        /// Issue ID.
        id: u64,
    },
}

/// Issue lifecycle state, as a CLI argument.
#[derive(Clone, Copy, Debug, clap::ValueEnum)]
pub enum StateArg {
    /// The issue is active and unresolved.
    Open,
    /// The issue has been resolved or won't be fixed.
    Closed,
}
