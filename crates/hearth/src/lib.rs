//! Environments as Git trees.
//!
//! Hearth is an environment manager backed by Git's content-addressed object
//! store. It treats environments as compositions of content-addressed filesystem
//! trees, providing reproducible, inspectable, and shareable environments with
//! graduated isolation.

#[cfg(test)]
mod tests;

pub mod cli;
pub mod env;
pub mod exe;
pub mod import;
pub mod store;

use std::fmt;

/// Ref prefix for imported component trees.
pub const TREES_REF_PREFIX: &str = "refs/hearth/trees/";

/// Ref prefix for merged environment trees.
pub const ENVS_REF_PREFIX: &str = "refs/hearth/envs/";

/// Ref prefix for VM kernel blobs.
pub const KERNELS_REF_PREFIX: &str = "refs/hearth/kernels/";

/// Errors produced by hearth operations.
#[derive(Debug)]
pub enum Error {
    /// A Git operation failed.
    Git(git2::Error),
    /// A filesystem operation failed.
    Io(std::io::Error),
    /// An environment configuration error.
    Config(String),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Git(e) => write!(f, "{e}"),
            Self::Io(e) => write!(f, "{e}"),
            Self::Config(msg) => write!(f, "{msg}"),
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Git(e) => Some(e),
            Self::Io(e) => Some(e),
            Self::Config(_) => None,
        }
    }
}

impl From<git2::Error> for Error {
    fn from(e: git2::Error) -> Self {
        Self::Git(e)
    }
}

impl From<std::io::Error> for Error {
    fn from(e: std::io::Error) -> Self {
        Self::Io(e)
    }
}
