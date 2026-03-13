//! General-purpose ordered queue refs under `refs/meta/queue/<name>`.
//!
//! The merge queue is one instance of this primitive:
//!
//! ```text
//! refs/meta/queue/merge → commit → tree
//! ├── 001-<review-id>     # toml: head_commit, submitted_by, timestamp
//! ├── 002-<review-id>
//! └── ...
//! ```
//!
//! Processing hooks declare what happens when entries appear. The merge
//! queue's hook rebases and tests. A CI queue's hook executes build actions.
//! A release pipeline chains queues.

pub mod git2;

/// The ref under which the merge queue is stored.
pub const MERGE_QUEUE_REF: &str = "refs/meta/queue/merge";

/// Ref prefix for named queues.
pub const QUEUE_REF_PREFIX: &str = "refs/meta/queue/";

/// A single entry in a named queue ref.
#[derive(Clone, Debug)]
pub struct QueueEntry {
    /// The sequential position key (e.g. `"001-<review-id>"`).
    pub key: String,
    /// The commit OID at the head of the branch when this entry was submitted.
    pub head_commit: ::git2::Oid,
    /// Fingerprint of the contributor who submitted this entry.
    pub submitted_by: String,
    /// RFC 3339 timestamp of submission.
    pub timestamp: String,
}

/// Parameters for pushing a new entry onto a queue.
#[derive(Clone, Debug)]
pub struct NewQueueEntry {
    /// The commit OID to enqueue.
    pub head_commit: ::git2::Oid,
    /// Fingerprint of the submitter.
    pub submitted_by: String,
}

/// Operations on named queue refs under [`QUEUE_REF_PREFIX`].
pub trait Queue {
    /// Return the ref name for a named queue.
    fn queue_ref(name: &str) -> String {
        format!("{QUEUE_REF_PREFIX}{name}")
    }

    /// Return all entries in the named queue, ordered by position ascending.
    fn list_queue(&self, name: &str) -> Result<Vec<QueueEntry>, ::git2::Error>;

    /// Return the first entry in the queue without removing it, or `None` if
    /// the queue is empty.
    fn peek_queue(&self, name: &str) -> Result<Option<QueueEntry>, ::git2::Error>;

    /// Append a new entry to the named queue, returning its assigned key.
    fn push_queue(&self, name: &str, entry: &NewQueueEntry) -> Result<String, ::git2::Error>;

    /// Remove and return the first entry from the named queue, or `None` if
    /// the queue is empty.
    fn pop_queue(&self, name: &str) -> Result<Option<QueueEntry>, ::git2::Error>;

    /// Remove the entry identified by `key` from the named queue regardless of
    /// its position. Used when an entry is rejected mid-queue.
    fn remove_queue_entry(&self, name: &str, key: &str) -> Result<(), ::git2::Error>;
}
