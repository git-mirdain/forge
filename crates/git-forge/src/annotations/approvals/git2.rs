//! `git2::Repository` implementation of [`Approvals`].

use git2::Repository;

use crate::annotations::approvals::{Approval, Approvals, NewApproval};

impl Approvals for Repository {
    fn approvals_for(&self, _object_id: &str) -> Result<Vec<Approval>, git2::Error> {
        todo!()
    }

    fn find_approval(
        &self,
        _object_id: &str,
        _approver_fingerprint: &str,
    ) -> Result<Option<Approval>, git2::Error> {
        todo!()
    }

    fn add_approval(&self, _approval: &NewApproval) -> Result<(), git2::Error> {
        todo!()
    }

    fn bulk_approve_range(
        &self,
        _patch_ids: &[String],
        _range_patch_id: &str,
        _message: Option<&str>,
    ) -> Result<(), git2::Error> {
        todo!()
    }

    fn approval_count_satisfies(
        &self,
        _object_id: &str,
        _min_approvals: usize,
        _exclude: Option<&str>,
    ) -> Result<bool, git2::Error> {
        todo!()
    }
}
