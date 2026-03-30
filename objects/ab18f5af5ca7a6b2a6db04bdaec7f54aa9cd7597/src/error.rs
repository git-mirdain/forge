//! Error types for the forge library.

/// Errors that can occur in forge operations.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// A git operation failed.
    #[error(transparent)]
    Git(#[from] git2::Error),
    /// An I/O operation failed.
    #[error(transparent)]
    Io(#[from] std::io::Error),
    /// No entity matches the given display ID or OID prefix.
    #[error("no entity matching #{0}")]
    NotFound(String),
    /// Multiple entities match the given OID prefix.
    #[error("ambiguous OID prefix #{0}")]
    Ambiguous(String),
    /// A field contains an unrecognized state value.
    #[error("invalid state: {0}")]
    InvalidState(String),
    /// The user cancelled an interactive prompt.
    #[error("interrupted")]
    Interrupted,
    /// A remote sync operation failed.
    #[error("sync: {0}")]
    Sync(String),
    /// The working tree or index has uncommitted changes.
    #[error("working tree is dirty; use --allow-dirty to proceed")]
    DirtyWorktree,
    /// The git object type is not valid for this operation.
    #[error("invalid object type for commenting: {0}; expected commit, blob, or tag")]
    InvalidObjectType(String),
    /// A configuration error.
    #[error("config: {0}")]
    Config(String),
}

/// Convenience result alias.
pub type Result<T> = std::result::Result<T, Error>;
