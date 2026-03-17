//! The CLI definitions for the top-level `git forge` command.

use clap::{Parser, Subcommand};
use git_forge_comment::cli::CommentCommand;
use git_forge_issue::cli::IssueCommand;
use git_forge_release::cli::ReleaseCommand;
use git_forge_review::cli::ReviewCommand;

/// Local-first infrastructure for Git forges.
#[derive(Parser)]
#[command(name = "git forge", bin_name = "git forge")]
#[command(author, version)]
pub struct Cli {
    /// Do not push forge refs to the remote after mutating.
    #[arg(long = "no-push", global = true)]
    pub no_push: bool,

    /// Do not fetch forge refs before mutating. Implied by --no-push.
    #[arg(long = "no-fetch", global = true)]
    pub no_fetch: bool,

    /// Fetch forge refs even when --no-push is set.
    #[arg(long = "fetch", global = true, conflicts_with = "no_fetch")]
    pub fetch: bool,

    /// The subcommand to run.
    #[command(subcommand)]
    pub command: Commands,
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
