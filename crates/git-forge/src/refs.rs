//! Ref prefix constants for the forge namespace.

/// Entity ref prefix for issues.
pub const ISSUE_PREFIX: &str = "refs/forge/issue/";
/// Entity ref prefix for reviews.
pub const REVIEW_PREFIX: &str = "refs/forge/review/";
/// Chain ref prefix for issue comments.
pub const ISSUE_COMMENTS_PREFIX: &str = "refs/forge/comments/issue/";
/// Chain ref prefix for review comments.
pub const REVIEW_COMMENTS_PREFIX: &str = "refs/forge/comments/review/";
/// Index ref mapping display IDs ↔ OIDs for issues.
pub const ISSUE_INDEX: &str = "refs/forge/meta/index/issues";
/// Index ref mapping display IDs ↔ OIDs for reviews.
pub const REVIEW_INDEX: &str = "refs/forge/meta/index/reviews";
