//! CLI definitions for `git forge release`.

use clap::Subcommand;

/// Subcommands for `git forge release`.
#[derive(Subcommand)]
pub enum ReleaseCommand {
    /// Create a new release.
    New,
    /// Edit an existing release.
    Edit,
    /// List releases.
    List,
    /// Show the status of a release.
    Status,
    /// Show details of a release.
    Show,
}
