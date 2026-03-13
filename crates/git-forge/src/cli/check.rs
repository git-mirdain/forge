//! CLI definitions for `git forge check`.

use clap::Subcommand;

/// Commands for `git forge check`.
#[derive(Subcommand)]
pub enum CheckCommand {
    /// Trigger a new check.
    New,
    /// Edit an existing check.
    Edit,
    /// List checks.
    List,
    /// Show the status of a check.
    Status,
    /// Show details of a check.
    Show,
}
