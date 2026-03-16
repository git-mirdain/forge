//! CLI definitions for `git forge comment`.

use clap::Subcommand;

/// Subcommands for `git forge comment`.
#[derive(Subcommand, Clone)]
pub enum CommentCommand {
    /// Add a new comment to any git object.
    New {
        /// Target: "issue/<id>", "review/<id>", "commit/<sha>", "blob/<sha>", etc.
        target: String,

        /// Comment body (markdown). Reads from stdin if omitted.
        #[arg(short, long)]
        body: Option<String>,

        /// Object SHA being commented on (defaults to HEAD commit).
        #[arg(long)]
        anchor: Option<String>,

        /// Anchor type: blob, commit, tree, or commit-range.
        #[arg(long)]
        anchor_type: Option<String>,

        /// Line range within a blob, e.g. "42-47".
        #[arg(long)]
        range: Option<String>,
    },

    /// Reply to an existing comment.
    Reply {
        /// Target: "issue/<id>", "review/<id>", "commit/<sha>", etc.
        target: String,

        /// OID of the comment to reply to.
        comment: String,

        /// Reply body (markdown). Reads from stdin if omitted.
        #[arg(short, long)]
        body: Option<String>,
    },

    /// Resolve a comment thread.
    Resolve {
        /// Target: "issue/<id>", "review/<id>", "commit/<sha>", etc.
        target: String,

        /// OID of the comment to resolve.
        comment: String,

        /// Optional resolution message.
        #[arg(short, long)]
        message: Option<String>,
    },

    /// List comments on a target.
    List {
        /// Target: "issue/<id>", "review/<id>", "commit/<sha>", etc.
        target: String,
    },

    /// Show a single comment in full.
    View {
        /// Target: "issue/<id>", "review/<id>", "commit/<sha>", etc.
        target: String,

        /// OID of the comment to view.
        comment: String,
    },
}
