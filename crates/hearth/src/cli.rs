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
        /// Environment name (defaults to project.default from config).
        env: Option<String>,

        /// Isolation level (0 = host, 1 = workspace, 2 = read-only).
        #[arg(long, default_value_t = 0)]
        isolation: u8,

        /// Path to .forge/environment.toml (default: ./.forge/environment.toml).
        #[arg(long, default_value = ".forge/environment.toml")]
        config: String,

        /// Path to .forge/toolchains.toml (default: ./.forge/toolchains.toml).
        #[arg(long, default_value = ".forge/toolchains.toml")]
        toolchains: String,
    },

    /// Run a command inside an environment and exit.
    Run {
        /// Environment name (defaults to project.default from config).
        #[arg(long)]
        env: Option<String>,

        /// Isolation level (0 = host, 1 = workspace, 2 = read-only).
        #[arg(long, default_value_t = 0)]
        isolation: u8,

        /// Path to .forge/environment.toml (default: ./.forge/environment.toml).
        #[arg(long, default_value = ".forge/environment.toml")]
        config: String,

        /// Path to .forge/toolchains.toml (default: ./.forge/toolchains.toml).
        #[arg(long, default_value = ".forge/toolchains.toml")]
        toolchains: String,

        /// The command and its arguments to run.
        #[arg(trailing_var_arg = true, required = true)]
        cmd: Vec<String>,
    },

    /// Print the merged environment hash without materializing.
    Hash {
        /// Environment name (defaults to project.default from config).
        env: Option<String>,

        /// Path to .forge/environment.toml (default: ./.forge/environment.toml).
        #[arg(long, default_value = ".forge/environment.toml")]
        config: String,

        /// Path to .forge/toolchains.toml (default: ./.forge/toolchains.toml).
        #[arg(long, default_value = ".forge/toolchains.toml")]
        toolchains: String,
    },

    /// Show differences between two environments.
    Diff {
        /// First environment name or tree hash.
        env_a: String,
        /// Second environment name or tree hash.
        env_b: String,

        /// Path to .forge/environment.toml (default: ./.forge/environment.toml).
        #[arg(long, default_value = ".forge/environment.toml")]
        config: String,

        /// Path to .forge/toolchains.toml (default: ./.forge/toolchains.toml).
        #[arg(long, default_value = ".forge/toolchains.toml")]
        toolchains: String,
    },

    /// Check out an environment tree to a path on disk.
    Checkout {
        /// Environment name (defaults to project.default from config) or tree hash.
        env: Option<String>,

        /// Destination path (default: store/<hash>/).
        #[arg(long)]
        path: Option<String>,

        /// Print direnv-compatible `export` lines instead of a path.
        #[arg(long)]
        direnv: bool,

        /// Path to .forge/environment.toml (default: ./.forge/environment.toml).
        #[arg(long, default_value = ".forge/environment.toml")]
        config: String,

        /// Path to .forge/toolchains.toml (default: ./.forge/toolchains.toml).
        #[arg(long, default_value = ".forge/toolchains.toml")]
        toolchains: String,
    },

    /// Garbage-collect unreferenced blobs from the cache.
    Gc,

    /// Show store status (imported trees, environments, cache size).
    Status,

    /// Track a toolchain in toolchains.toml.
    Track {
        /// Toolchain name.
        name: String,

        /// Source URI or local path to import, or an existing tree OID.
        source: String,

        /// Strip N leading path components from tarball entries.
        #[arg(long, default_value_t = 0)]
        strip_prefix: usize,

        /// Path to .forge/toolchains.toml.
        #[arg(long, default_value = ".forge/toolchains.toml")]
        toolchains: String,
    },

    /// Remove a toolchain from toolchains.toml.
    Untrack {
        /// Toolchain name to remove.
        name: String,

        /// Path to .forge/toolchains.toml.
        #[arg(long, default_value = ".forge/toolchains.toml")]
        toolchains: String,
    },
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
