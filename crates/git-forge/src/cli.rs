//! The CLI definitions for the top-level `git forge` command.

use clap::{Parser, Subcommand};
use git_forge_issues::cli::IssueCommand;
use git_forge_release::cli::ReleaseCommand;
use git_forge_review::cli::ReviewCommand;

/// Local-first infrastructure for Git forges.
#[derive(Parser)]
#[command(name = "git forge", bin_name = "git forge")]
#[command(author, version)]
pub struct Cli {
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
}
