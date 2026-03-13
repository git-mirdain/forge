//! The CLI definitions for the top-level `git forge` command.

pub mod check;
pub mod issue;
pub mod review;

use clap::{Parser, Subcommand};

/// Local-first infrastructure for Git forges.
#[derive(Parser)]
#[command(name = "git forge", bin_name = "git forge")]
#[command(author, version)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Commands,
}

/// Top-level subcommands.
#[derive(Subcommand)]
pub enum Commands {
    /// Work with issues.
    Issue {
        #[command(subcommand)]
        command: issue::IssueCommand,
    },
    /// Work with pull/merge request reviews.
    Review {
        #[command(subcommand)]
        command: review::ReviewCommand,
    },
    /// Work with CI checks.
    Check {
        #[command(subcommand)]
        command: check::CheckCommand,
    },
}
