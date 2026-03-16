//! Code, review, and issue comments anchored to Git objects.
//!
//! A comment is stored as a Git commit object. Trailers on the commit message
//! carry structured metadata (anchor, resolved state, parent comment OID, etc.).
//!
//! Refs live under `refs/forge/comments/`.

pub mod cli;
pub mod exe;
pub mod git2;

#[cfg(test)]
mod tests;

/// Ref prefix under which comment refs are stored.
pub const COMMENTS_REF_PREFIX: &str = "refs/forge/comments/";

/// Returns the ref name for comments on a specific issue.
#[must_use]
pub fn issue_comments_ref(id: u64) -> String {
    format!("{COMMENTS_REF_PREFIX}issue/{id}")
}

/// Returns the ref name for comments on a specific review (pull request).
#[must_use]
pub fn review_comments_ref(id: u64) -> String {
    format!("{COMMENTS_REF_PREFIX}review/{id}")
}

/// The location within a Git object that a comment targets.
#[derive(Clone, Debug)]
pub enum Anchor {
    /// A blob (file), with zero or more line ranges (union).
    Blob {
        /// SHA of the blob object.
        oid: ::git2::Oid,
        /// Line ranges within the blob; empty means the whole file.
        line_ranges: Vec<(u32, u32)>,
    },
    /// A single commit.
    Commit(::git2::Oid),
    /// A tree (directory).
    Tree(::git2::Oid),
    /// A range between two commits (inclusive).
    CommitRange {
        /// SHA of the first commit in the range.
        start: ::git2::Oid,
        /// SHA of the last commit in the range.
        end: ::git2::Oid,
    },
}

/// A comment stored as a commit under `refs/forge/comments/`.
///
/// Author identity and timestamp are read from the commit's author field directly.
#[derive(Clone, Debug)]
pub struct Comment {
    /// OID of the commit that represents this comment.
    pub oid: ::git2::Oid,
    /// What this comment is anchored to.
    pub anchor: Anchor,
    /// Markdown body (the commit message, trailers stripped).
    pub body: String,
    /// Whether the thread has been resolved (`Resolved: true` trailer).
    pub resolved: bool,
    /// OID of the parent comment (second parent), if this is a reply.
    pub parent_oid: Option<::git2::Oid>,
    /// OID of the comment this replaces (`Replaces: <oid>` trailer), if this is an edit.
    pub replaces_oid: Option<::git2::Oid>,
}

/// Operations on comment refs under [`COMMENTS_REF_PREFIX`].
pub trait Comments {
    /// Return all comments on the given ref in reverse-chronological order.
    ///
    /// # Errors
    ///
    /// Returns `git2::Error` if the underlying repository operation fails.
    fn comments_on(&self, ref_name: &str) -> Result<Vec<Comment>, ::git2::Error>;

    /// Find a single comment by OID, returning `None` if not found.
    ///
    /// # Errors
    ///
    /// Returns `git2::Error` if the underlying repository operation fails.
    fn find_comment(
        &self,
        ref_name: &str,
        oid: ::git2::Oid,
    ) -> Result<Option<Comment>, ::git2::Error>;

    /// Append a top-level comment to the chain, returning the new commit OID.
    ///
    /// # Errors
    ///
    /// Returns `git2::Error` if the underlying repository operation fails.
    fn add_comment(
        &self,
        ref_name: &str,
        anchor: &Anchor,
        body: &str,
    ) -> Result<::git2::Oid, ::git2::Error>;

    /// Append a reply to an existing comment, returning the new commit OID.
    ///
    /// # Errors
    ///
    /// Returns `git2::Error` if the underlying repository operation fails.
    fn reply_to_comment(
        &self,
        ref_name: &str,
        parent_oid: ::git2::Oid,
        body: &str,
    ) -> Result<::git2::Oid, ::git2::Error>;

    /// Append a resolution to an existing comment thread, returning the new commit OID.
    ///
    /// # Errors
    ///
    /// Returns `git2::Error` if the underlying repository operation fails.
    fn resolve_comment(
        &self,
        ref_name: &str,
        comment_oid: ::git2::Oid,
    ) -> Result<::git2::Oid, ::git2::Error>;

    /// Append an edit to an existing comment, returning the new commit OID.
    ///
    /// The original comment is unchanged. The new commit carries a `Replaces: <oid>`
    /// trailer pointing at the original and uses the original's anchor.
    ///
    /// # Errors
    ///
    /// Returns `git2::Error` if the underlying repository operation fails.
    fn edit_comment(
        &self,
        ref_name: &str,
        comment_oid: ::git2::Oid,
        new_body: &str,
    ) -> Result<::git2::Oid, ::git2::Error>;
}
