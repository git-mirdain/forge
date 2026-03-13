//! Entity ID counters backed by `refs/meta/counters`.
//!
//! Sequential integer IDs are assigned per entity type. A counter ref tracks
//! the next available ID:
//!
//! ```text
//! refs/meta/counters → commit → tree
//! ├── issues      # plain text: "47"
//! └── reviews     # plain text: "103"
//! ```
//!
//! Incrementing is a signed commit to this ref. The counter's history is its
//! own audit log.

pub mod git2;

/// The ref under which all counters are stored.
pub const COUNTERS_REF: &str = "refs/meta/counters";

/// Entity types that use sequential integer IDs.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum EntityKind {
    /// Issues (`refs/meta/issues/<id>`).
    Issue,
    /// Reviews (`refs/meta/reviews/<id>`).
    Review,
}

impl EntityKind {
    /// The filename used for this kind's counter in the counters tree.
    #[must_use]
    pub fn counter_name(self) -> &'static str {
        match self {
            Self::Issue => "issues",
            Self::Review => "reviews",
        }
    }
}

/// Operations on the entity ID counter ref.
///
/// All counter mutations are CAS-style commits so that parallel writers
/// serialise correctly. See the design specification's "Assignment Protocols"
/// section for the three supported protocols (pure Git, server hooks, UI
/// server direct access).
pub trait EntityCounter {
    /// Read the current value of `kind`'s counter without incrementing it.
    ///
    /// Returns `0` when no counter has been written yet (fresh repository).
    fn read_counter(&self, kind: EntityKind) -> Result<u64, ::git2::Error>;

    /// Atomically increment `kind`'s counter and return the newly assigned ID.
    ///
    /// Uses optimistic concurrency: reads the current value, prepares a new
    /// commit, and pushes with `--force-with-lease` semantics. Callers should
    /// retry on conflict (`git2::ErrorCode::NotFastForward`).
    fn increment_counter(&self, kind: EntityKind) -> Result<u64, ::git2::Error>;
}
