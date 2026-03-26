//! The CLI definitions for the top-level `forge` command.

use clap::{Parser, Subcommand};
use git_forge_comment::cli::CommentCommand;
use git_forge_issue::cli::IssueCommand;
use git_forge_release::cli::ReleaseCommand;
use git_forge_review::cli::ReviewCommand;

/// Local-first infrastructure for Git forges.
#[derive(Parser)]
#[command(name = "forge", bin_name = "forge")]
#[command(author, version)]
pub struct Cli {
    /// The subcommand to run.
    #[command(subcommand)]
    pub command: Commands,
}

/// Subcommands for `git forge contributor`.
#[derive(Subcommand, Clone)]
pub enum ContributorSubcommand {
    /// Add a contributor to the registry.
    Add {
        /// Contributor ID. Derived from the first word of name if omitted.
        #[arg(long)]
        id: Option<String>,

        /// Display name. Defaults to git config user.name.
        #[arg(long)]
        name: Option<String>,

        /// Email address. May be repeated for multiple addresses.
        /// Defaults to git config user.email if none provided.
        #[arg(long = "email")]
        emails: Vec<String>,
    },
    /// Edit an existing contributor.
    Edit {
        /// Contributor ID to edit.
        id: String,

        /// New ID (renames the contributor).
        #[arg(long = "rename-id")]
        new_id: Option<String>,

        /// New display name.
        #[arg(long)]
        name: Option<String>,

        /// Email address to add. May be repeated.
        #[arg(long = "add-email")]
        add_emails: Vec<String>,

        /// Email address to remove. May be repeated.
        #[arg(long = "remove-email")]
        remove_emails: Vec<String>,
    },
    /// List all contributors.
    List,
    /// Remove a contributor from the registry.
    Remove {
        /// Contributor ID to remove.
        id: String,
    },
    /// Show details for a contributor.
    Show {
        /// Contributor ID to look up.
        id: String,
    },
}

/// Top-level subcommands.
#[derive(Subcommand)]
pub enum Commands {
    /// Work with issues.
    Issue {
        /// The issue subcommand to run.
        #[command(subcommand)]
        command: IssueCommand,
    },
    /// Work with pull/merge request reviews.
    Review {
        /// The review subcommand to run.
        #[command(subcommand)]
        command: ReviewCommand,
    },
    /// Work with releases.
    Release {
        /// The release subcommand to run.
        #[command(subcommand)]
        command: ReleaseCommand,
    },
    /// Work with comments on issues and reviews.
    Comment {
        /// The comment subcommand to run.
        #[command(subcommand)]
        command: CommentCommand,
    },

    /// Manage contributors.
    Contributor {
        /// The contributor subcommand to run.
        #[command(subcommand)]
        command: ContributorSubcommand,
    },

    /// Install forge refspecs into git config for a remote.
    Install {
        /// Remote to configure. Defaults to `origin` if it exists.
        remote: Option<String>,

        /// Add the refspec to the global git config (~/.gitconfig) instead of the local repo config.
        #[arg(long)]
        global: bool,
    },

    /// Sync forge refs with a remote (fetch + push). Respects --no-fetch and --no-push.
    Sync {
        /// Remote to sync with. Defaults to `origin`.
        remote: Option<String>,
    },
}
