//! Local-first infrastructure for Git forges.

pub mod error;
pub mod issue;
pub mod refs;

#[cfg(feature = "cli")]
pub mod cli;

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
}

#[cfg(test)]
mod tests;
