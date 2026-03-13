//! `git2::Repository` implementation of [`Contributors`].

use git2::Repository;

use crate::access::contributors::{Contributor, Contributors, NewContributor};

impl Contributors for Repository {
    fn list_contributors(&self) -> Result<Vec<Contributor>, git2::Error> {
        todo!()
    }

    fn find_contributor(&self, _fingerprint: &str) -> Result<Option<Contributor>, git2::Error> {
        todo!()
    }

    fn add_contributor(&self, _contributor: &NewContributor) -> Result<(), git2::Error> {
        todo!()
    }

    fn remove_contributor(&self, _fingerprint: &str) -> Result<(), git2::Error> {
        todo!()
    }

    fn set_contributor_roles(
        &self,
        _fingerprint: &str,
        _roles: &[String],
    ) -> Result<(), git2::Error> {
        todo!()
    }

    fn contributor_has_role(
        &self,
        _fingerprint: &str,
        _roles: &[&str],
    ) -> Result<bool, git2::Error> {
        todo!()
    }
}
