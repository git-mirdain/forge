//! Local-first infrastructure for Git forges.

pub mod actor;
pub mod comment;
pub mod error;
pub mod issue;
pub mod refs;
pub mod review;
pub mod sync;

#[cfg(feature = "cli")]
pub mod cli;

#[cfg(feature = "cli")]
pub mod input;

#[cfg(feature = "cli")]
pub mod interactive;

#[cfg(feature = "exe")]
pub mod exe;

mod index;
pub(crate) mod reindex;

pub use error::{Error, Result};

use git_ledger::{Ledger, Mutation};
use git2::Repository;

// TODO: `RemoteSync` takes `&Repository` instead of `&Store`, breaking
// encapsulation — the sync trait can bypass Store invariants. Migrating
// to `&Store` would also require `Send` bounds on the returned futures
// to support concurrent sync operations.

/// A handle to the forge store in a Git repository.
pub struct Store<'a> {
    pub(crate) repo: &'a Repository,
}

impl<'a> Store<'a> {
    /// Open a store backed by the given repository.
    #[must_use]
    pub fn new(repo: &'a Repository) -> Self {
        Self { repo }
    }

    /// Write a display ID alias into a ledger entity index.
    ///
    /// `index_ref` is the ref that backs the index (e.g. [`refs::ISSUE_INDEX`]).
    ///
    /// # Errors
    /// Returns an error if a git operation fails.
    pub fn write_display_id(&self, index_ref: &str, display_id: &str, oid: &str) -> Result<()> {
        index::index_upsert(self.repo, index_ref, &[(display_id, oid)])
    }

    /// Write a `source/url` field to an existing ledger entry.
    ///
    /// `ref_prefix` is e.g. [`refs::ISSUE_PREFIX`] or [`refs::REVIEW_PREFIX`].
    ///
    /// # Errors
    /// Returns an error if a git operation fails.
    pub fn write_source_url(&self, ref_prefix: &str, oid: &str, url: &str) -> Result<()> {
        let ref_name = format!("{ref_prefix}{oid}");
        self.repo.update(
            &ref_name,
            &[Mutation::Set("source/url", url.as_bytes())],
            "forge: set source url",
        )?;
        Ok(())
    }
}

#[cfg(test)]
mod tests;
