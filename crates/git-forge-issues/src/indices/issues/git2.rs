//! `git2::Repository` implementation of [`Issues`].

use git2::Repository;

use crate::indices::issues::{Issue, IssueState, IssueUpdate, Issues, NewIssue};

impl Issues for Repository {
    fn list_issues(&self) -> Result<Vec<Issue>, git2::Error> {
        todo!()
    }

    fn list_issues_by_state(&self, _state: IssueState) -> Result<Vec<Issue>, git2::Error> {
        todo!()
    }

    fn find_issue(&self, _id: u64) -> Result<Option<Issue>, git2::Error> {
        todo!()
    }

    fn create_issue(&self, _issue: &NewIssue) -> Result<u64, git2::Error> {
        todo!()
    }

    fn update_issue(&self, _id: u64, _update: &IssueUpdate) -> Result<(), git2::Error> {
        todo!()
    }

    fn add_issue_comment(&self, _id: u64, _author: &str, _body: &str) -> Result<(), git2::Error> {
        todo!()
    }
}
