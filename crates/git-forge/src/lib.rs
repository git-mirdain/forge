//! Local-first infrastructure for Git forges.

pub mod cli;

pub use git_forge_core as core;
pub use git_forge_issues as issues;
pub use git_forge_release as release;
pub use git_forge_review as review;
