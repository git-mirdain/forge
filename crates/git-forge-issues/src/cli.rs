//! CLI definitions for `git forge issue`.

use clap::Subcommand;

/// Subcommands for `git forge issue`.
#[derive(Subcommand)]
pub enum IssueCommand {
    /// Open a new issue.
    New,
    /// Edit an existing issue.
    Edit,
    /// List issues.
    List,
    /// Show the status of an issue.
    Status,
    /// Show details of an issue.
    Show,
}
