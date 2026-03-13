//! `git2::Repository` implementation of [`AccessPolicy`].

use git2::Repository;

use crate::access::policy::{AccessPolicy, BranchPolicy, Policy, RolePermissions};

impl AccessPolicy for Repository {
    fn load_policy(&self) -> Result<Policy, git2::Error> {
        todo!()
    }

    fn load_policy_at(&self, _commit_oid: git2::Oid) -> Result<Policy, git2::Error> {
        todo!()
    }

    fn branch_policy(&self, _branch_ref: &str) -> Result<Option<BranchPolicy>, git2::Error> {
        todo!()
    }

    fn role_permissions(&self, _role_name: &str) -> Result<Option<RolePermissions>, git2::Error> {
        todo!()
    }

    fn check_push_permission(
        &self,
        _fingerprint: &str,
        _target_ref: &str,
        _changed_paths: &[&str],
    ) -> Result<bool, git2::Error> {
        todo!()
    }
}
