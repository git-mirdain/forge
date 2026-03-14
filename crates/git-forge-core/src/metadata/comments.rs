//! Code comments anchored to blob OIDs and line ranges.
//!
//! Comments are stored as `git-metadata` entries on blob OIDs:
//!
//! ```text
//! refs/metadata/comments   (fanout by blob oid)
//!   <blob-oid>/
//!     <comment-id>/
//!       meta          # toml: author, timestamp, start_line,
//!                     #       end_line, context_lines
//!       body          # markdown
//!       resolved      # toml: by, timestamp — presence means resolved
//!       reply/
//!         001         # toml: author, timestamp, body
//!         002
//! ```
//!
//! Comments are repo-wide. They are not owned by a review or issue. A review
//! may prompt someone to leave a comment, but the comment exists independently
//! and persists as long as the code it describes exists.

pub mod git2;

/// The ref under which all comment metadata is stored (fanout by blob OID).
pub const COMMENTS_REF: &str = "refs/metadata/comments";

/// A line-range anchor within a blob.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LineRange {
    /// First line of the anchored range (1-based, inclusive).
    pub start: u32,
    /// Last line of the anchored range (1-based, inclusive).
    pub end: u32,
}

/// Resolution metadata written to `<comment-id>/resolved`.
///
/// Presence of this entry means the comment is resolved.
#[derive(Clone, Debug)]
pub struct Resolution {
    /// Fingerprint of the contributor who resolved the comment.
    pub by: String,
    /// RFC 3339 timestamp of resolution.
    pub timestamp: String,
}

/// A reply within a comment thread.
#[derive(Clone, Debug)]
pub struct Reply {
    /// Sequential index (e.g. `"001"`, `"002"`).
    pub index: String,
    /// Fingerprint of the reply author.
    pub author: String,
    /// RFC 3339 timestamp.
    pub timestamp: String,
    /// Markdown body.
    pub body: String,
}

/// A comment anchored to a specific blob OID and line range.
#[derive(Clone, Debug)]
pub struct Comment {
    /// Stable identifier for this comment (unique within the blob).
    pub id: String,
    /// The blob OID the comment is currently anchored to.
    pub blob_oid: ::git2::Oid,
    /// The line range within the blob.
    pub range: LineRange,
    /// A few lines of surrounding context used as a fallback for reanchoring
    /// when blame is ambiguous.
    pub context_lines: Vec<String>,
    /// Fingerprint of the comment author.
    pub author: String,
    /// RFC 3339 creation timestamp.
    pub timestamp: String,
    /// Markdown body.
    pub body: String,
    /// Resolution state. `None` means open.
    pub resolved: Option<Resolution>,
    /// Threaded replies, ordered chronologically.
    pub replies: Vec<Reply>,
}

/// Parameters for leaving a new comment.
#[derive(Clone, Debug)]
pub struct NewComment {
    /// The blob OID to anchor the comment to.
    pub blob_oid: ::git2::Oid,
    /// The line range within the blob.
    pub range: LineRange,
    /// Lines of surrounding context for reanchoring fallback.
    pub context_lines: Vec<String>,
    /// Markdown body.
    pub body: String,
}

/// Parameters for reanchoring a comment to a new blob after the file changed.
#[derive(Clone, Debug)]
pub struct Reanchor {
    /// The comment ID being reanchored.
    pub comment_id: String,
    /// The old blob OID the comment was anchored to.
    pub old_blob_oid: ::git2::Oid,
    /// The new blob OID to anchor to.
    pub new_blob_oid: ::git2::Oid,
    /// The updated line range in the new blob.
    pub new_range: LineRange,
}

/// Operations on code comments stored under [`COMMENTS_REF`].
pub trait Comments {
    /// Return all comments anchored to `blob_oid`.
    fn comments_on_blob(&self, blob_oid: ::git2::Oid) -> Result<Vec<Comment>, ::git2::Error>;

    /// Return the comment with `id` anchored to `blob_oid`, or `None` if it
    /// does not exist.
    fn find_comment(
        &self,
        blob_oid: ::git2::Oid,
        id: &str,
    ) -> Result<Option<Comment>, ::git2::Error>;

    /// Leave a new comment, returning the assigned comment ID.
    fn add_comment(&self, comment: &NewComment) -> Result<String, ::git2::Error>;

    /// Add a reply to an existing comment thread.
    fn reply_to_comment(
        &self,
        blob_oid: ::git2::Oid,
        comment_id: &str,
        author: &str,
        body: &str,
    ) -> Result<(), ::git2::Error>;

    /// Mark a comment as resolved.
    fn resolve_comment(
        &self,
        blob_oid: ::git2::Oid,
        comment_id: &str,
        resolver: &str,
    ) -> Result<(), ::git2::Error>;

    /// Reanchor a comment from its current blob OID to a new one after the
    /// file was modified. Writes a new metadata commit recording the updated
    /// anchor.
    fn reanchor_comment(&self, reanchor: &Reanchor) -> Result<(), ::git2::Error>;

    /// Return all comments whose blob OID no longer appears in any file
    /// reachable from `HEAD`. These are "orphaned" comments.
    fn orphaned_comments(&self) -> Result<Vec<Comment>, ::git2::Error>;
}
