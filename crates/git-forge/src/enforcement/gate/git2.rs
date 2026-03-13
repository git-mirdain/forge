//! `git2::Repository` implementation of [`MergeGate`].

use git2::Repository;

use crate::enforcement::gate::{GateOutcome, MergeGate, PushEvent};

impl MergeGate for Repository {
    fn evaluate_push(&self, _event: &PushEvent) -> Result<GateOutcome, git2::Error> {
        todo!()
    }
}
