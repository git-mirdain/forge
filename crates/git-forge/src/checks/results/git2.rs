//! `git2::Repository` implementation of [`CheckResults`].

use git2::Repository;

use crate::checks::results::{CheckResult, CheckResults, NewCheckResult};

impl CheckResults for Repository {
    fn check_results_for(&self, _commit_oid: git2::Oid) -> Result<Vec<CheckResult>, git2::Error> {
        todo!()
    }

    fn latest_check_results(
        &self,
        _commit_oid: git2::Oid,
    ) -> Result<Vec<CheckResult>, git2::Error> {
        todo!()
    }

    fn latest_check_result(
        &self,
        _commit_oid: git2::Oid,
        _name: &str,
    ) -> Result<Option<CheckResult>, git2::Error> {
        todo!()
    }

    fn record_check_result(&self, _result: &NewCheckResult) -> Result<String, git2::Error> {
        todo!()
    }

    fn required_checks_pass(
        &self,
        _commit_oid: git2::Oid,
        _required: &[&str],
    ) -> Result<bool, git2::Error> {
        todo!()
    }
}
