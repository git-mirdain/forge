//! CLI definitions for `hearth`.

use clap::{Parser, Subcommand};

/// Environments as Git trees.
#[derive(Parser)]
#[command(name = "hearth", version, about)]
pub struct Cli {
    /// Path to the hearth store (default: ~/.hearth).
    #[arg(long, global = true, env = "HEARTH_STORE")]
    pub store: Option<String>,

    /// Subcommand to execute.
    #[command(subcommand)]
    pub command: Command,
}

/// Top-level subcommands.
#[derive(Subcommand)]
pub enum Command {
    /// Import a tree into the store.
    #[command(subcommand)]
    Import(ImportCommand),

    /// Enter an environment interactively by spawning a shell inside it.
    Enter {
        /// Environment name (from env.toml).
        env: String,

        /// Isolation level (0 = convention only, 1 = read-only inputs).
        #[arg(long, default_value_t = 0)]
        isolation: u8,

        /// Path to env.toml (default: ./env.toml).
        #[arg(long, default_value = "env.toml")]
        config: String,
    },

    /// Print the merged environment hash without materializing.
    Hash {
        /// Environment name (from env.toml).
        env: String,

        /// Path to env.toml (default: ./env.toml).
        #[arg(long, default_value = "env.toml")]
        config: String,
    },

    /// Show differences between two environments.
    Diff {
        /// First environment name or tree hash.
        env_a: String,
        /// Second environment name or tree hash.
        env_b: String,

        /// Path to env.toml (default: ./env.toml).
        #[arg(long, default_value = "env.toml")]
        config: String,
    },

    /// Check out an environment tree to a path on disk.
    Checkout {
        /// Environment name (from env.toml) or tree hash.
        env: String,

        /// Destination path (default: store/<hash>/).
        #[arg(long)]
        path: Option<String>,

        /// Print direnv-compatible `export` lines instead of a path.
        #[arg(long)]
        direnv: bool,

        /// Path to env.toml (default: ./env.toml).
        #[arg(long, default_value = "env.toml")]
        config: String,
    },

    /// Garbage-collect unreferenced blobs from the cache.
    Gc,

    /// Show store status (imported trees, environments, cache size).
    Status,
}

/// Import sub-subcommands.
#[derive(Subcommand)]
pub enum ImportCommand {
    /// Import a local directory.
    Dir {
        /// Path to the directory to import.
        path: String,
    },

    /// Import a tarball (.tar or .tar.gz).
    Tarball {
        /// Path to the tarball.
        path: String,

        /// Strip N leading path components from each entry.
        #[arg(long, default_value_t = 0)]
        strip_prefix: usize,
    },

    /// Import an OCI image (not yet implemented).
    Oci {
        /// OCI image reference.
        image_ref: String,
    },
}
