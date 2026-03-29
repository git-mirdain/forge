//! Local-first infrastructure for Git forges.

pub mod comment;
pub mod error;
pub mod issue;
pub mod refs;
pub mod sync;

#[cfg(feature = "cli")]
pub mod cli;

#[cfg(feature = "cli")]
pub mod interactive;

#[cfg(feature = "exe")]
pub mod exe;

mod index;

pub use error::{Error, Result};

use git2::Repository;

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
}

#[cfg(test)]
mod tests;
