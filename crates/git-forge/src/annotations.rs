//! Annotations on Git objects: comments and approvals.
//!
//! Annotations are metadata attached to Git objects via `git-metadata`. They
//! are independent of any entity (issue, review) that may have prompted them
//! and persist as long as the objects they describe exist.
//!
//! - [`comments`] — code comments anchored to blob OIDs and line ranges.
//! - [`approvals`] — approvals on blobs, trees, patch-ids, and range patch-ids.

pub mod approvals;
pub mod comments;
