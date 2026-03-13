//! The CLI definitions for the top-level `git kiln` command.

use clap::Parser;

/// A ceramic-inspired toolkit for shaping Git-native data.
#[derive(Parser)]
#[command(name = "git kiln", bin_name = "git kiln")]
#[command(author, version)]
pub struct Cli {}
