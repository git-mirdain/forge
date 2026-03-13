//! `git2::Repository` implementation of [`CheckDefinitions`].

use git2::Repository;

use crate::checks::definitions::{CheckDefinition, CheckDefinitions};

impl CheckDefinitions for Repository {
    fn list_check_definitions(&self) -> Result<Vec<CheckDefinition>, git2::Error> {
        todo!()
    }

    fn find_check_definition(&self, _name: &str) -> Result<Option<CheckDefinition>, git2::Error> {
        todo!()
    }

    fn find_check_definition_at(
        &self,
        _name: &str,
        _commit_oid: git2::Oid,
    ) -> Result<Option<CheckDefinition>, git2::Error> {
        todo!()
    }
}
