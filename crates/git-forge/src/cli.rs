//! The CLI definitions for the top-level `git forge` command.

use clap::Parser;

/// Local-first infrastructure for Git forges.
#[derive(Parser)]
#[command(name = "git forge", bin_name = "git forge")]
#[command(author, version)]
pub struct Cli {}
