//! Remote sync interface.
//!
//! Defines the trait that remote adapters (e.g. GitHub, GitLab) implement to
//! synchronize forge entities with an external host.

use git2::Repository;

use crate::Result;

/// Summary of a single sync run.
#[derive(Debug, Default)]
pub struct SyncReport {
    /// Number of entities newly imported.
    pub imported: usize,
    /// Number of entities exported to the remote.
    pub exported: usize,
    /// Number of entities skipped (already in sync).
    pub skipped: usize,
    /// Number of entities that failed.
    pub failed: usize,
}

/// A remote adapter that can import and export forge entities.
pub trait RemoteSync {
    /// Import issues from the remote into the local forge store.
    fn import_issues(
        &self,
        repo: &Repository,
    ) -> impl std::future::Future<Output = Result<SyncReport>>;

    /// Export locally-created issues to the remote.
    fn export_issues(
        &self,
        repo: &Repository,
    ) -> impl std::future::Future<Output = Result<SyncReport>>;
}
