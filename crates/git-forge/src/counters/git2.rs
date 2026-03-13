//! `git2::Repository` implementation of [`EntityCounter`].

use git2::Repository;

use crate::counters::{EntityCounter, EntityKind};

impl EntityCounter for Repository {
    fn read_counter(&self, _kind: EntityKind) -> Result<u64, git2::Error> {
        todo!()
    }

    fn increment_counter(&self, _kind: EntityKind) -> Result<u64, git2::Error> {
        todo!()
    }
}
