//! `git2::Repository` implementation of [`Issues`].

use git2::Repository;

use crate::issues::{Issue, IssueOpts, IssueState, Issues};

impl Issues for Repository {
    fn list_issues(&self, _opts: Option<&IssueOpts>) -> Result<Vec<Issue>, git2::Error> {
        todo!()
    }

    fn list_issues_by_state(
        &self,
        _state: IssueState,
        _opts: Option<&IssueOpts>,
    ) -> Result<Vec<Issue>, git2::Error> {
        todo!()
    }

    fn find_issue(
        &self,
        _id: u64,
        _opts: Option<&IssueOpts>,
    ) -> Result<Option<Issue>, git2::Error> {
        todo!()
    }

    fn create_issue(
        &self,
        _title: &str,
        _body: &str,
        _labels: &[String],
        _assignees: &[String],
        _opts: Option<&IssueOpts>,
    ) -> Result<u64, git2::Error> {
        todo!()
    }

    fn update_issue(
        &self,
        _id: u64,
        _title: Option<&str>,
        _body: Option<&str>,
        _labels: Option<&[String]>,
        _assignees: Option<&[String]>,
        _state: Option<IssueState>,
        _opts: Option<&IssueOpts>,
    ) -> Result<(), git2::Error> {
        todo!()
    }

    fn add_issue_comment(
        &self,
        _id: u64,
        _author: &str,
        _body: &str,
        _opts: Option<&IssueOpts>,
    ) -> Result<(), git2::Error> {
        todo!()
    }
}
