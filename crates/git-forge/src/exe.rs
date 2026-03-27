//! Executor for forge commands.
//!
//! [`Executor`] owns a [`git2::Repository`] and exposes typed methods for each
//! forge operation. The `run` method (available with the `cli` feature) dispatches
//! from a parsed [`crate::cli::Cli`] and writes output to stdout.

use std::path::Path;

use git2::Repository;

use crate::issue::{Issue, IssueState};
use crate::{Result, Store};

/// Owns a [`Repository`] and executes forge operations.
pub struct Executor {
    repo: Repository,
}

impl Executor {
    /// Discover the repository from the current directory.
    ///
    /// # Errors
    /// Returns an error if no git repository can be found.
    pub fn discover() -> Result<Self> {
        Ok(Self {
            repo: Repository::discover(".")?,
        })
    }

    /// Open the repository at `path`.
    ///
    /// # Errors
    /// Returns an error if the path is not a git repository.
    pub fn from_path(path: &Path) -> Result<Self> {
        Ok(Self {
            repo: Repository::open(path)?,
        })
    }

    fn store(&self) -> Store<'_> {
        Store::new(&self.repo)
    }

    /// Create a new issue.
    ///
    /// # Errors
    /// Returns an error if a git operation fails.
    pub fn create_issue(
        &self,
        title: &str,
        body: &str,
        labels: &[&str],
        assignees: &[&str],
    ) -> Result<Issue> {
        self.store().create_issue(title, body, labels, assignees)
    }

    /// Fetch an issue by display ID or OID prefix.
    ///
    /// # Errors
    /// Returns [`crate::Error::NotFound`] if no matching issue exists.
    pub fn get_issue(&self, reference: &str) -> Result<Issue> {
        self.store().get_issue(reference)
    }

    /// List all issues, optionally filtered by state.
    ///
    /// # Errors
    /// Returns an error if a git operation fails.
    pub fn list_issues(&self, state: Option<&IssueState>) -> Result<Vec<Issue>> {
        match state {
            Some(s) => self.store().list_issues_by_state(s),
            None => self.store().list_issues(),
        }
    }

    /// Apply a partial update to an issue.
    ///
    /// # Errors
    /// Returns [`crate::Error::NotFound`] if the issue does not exist.
    #[allow(clippy::too_many_arguments)]
    pub fn update_issue(
        &self,
        reference: &str,
        title: Option<&str>,
        body: Option<&str>,
        state: Option<&IssueState>,
        add_labels: &[&str],
        remove_labels: &[&str],
        add_assignees: &[&str],
        remove_assignees: &[&str],
    ) -> Result<Issue> {
        self.store().update_issue(
            reference,
            title,
            body,
            state,
            add_labels,
            remove_labels,
            add_assignees,
            remove_assignees,
        )
    }
}

#[cfg(feature = "cli")]
impl Executor {
    /// Dispatch a parsed CLI command, writing output to stdout.
    ///
    /// # Errors
    /// Returns an error if the underlying forge operation fails.
    ///
    /// # Panics
    /// Panics if facet-json fails to serialize a value (indicates a bug).
    pub fn run(&self, cli: &crate::cli::Cli) -> Result<()> {
        use crate::cli::{Command, IssueCommand};

        match &cli.command {
            Command::Issue { command } => match command {
                IssueCommand::New {
                    title,
                    body,
                    labels,
                    assignees,
                } => {
                    let labels: Vec<&str> = labels.iter().map(String::as_str).collect();
                    let assignees: Vec<&str> = assignees.iter().map(String::as_str).collect();
                    let issue = self.create_issue(
                        title,
                        body.as_deref().unwrap_or(""),
                        &labels,
                        &assignees,
                    )?;
                    print_issue(&issue, cli.json);
                }

                IssueCommand::Show { reference } => {
                    let issue = self.get_issue(reference)?;
                    print_issue(&issue, cli.json);
                }

                IssueCommand::List { state } => {
                    let issues = self.list_issues(state.as_ref())?;
                    if cli.json {
                        println!(
                            "{}",
                            facet_json::to_string_pretty(&issues).expect("serialize")
                        );
                    } else {
                        print_issue_list(&issues);
                    }
                }

                IssueCommand::Edit {
                    reference,
                    title,
                    body,
                    state,
                    add_labels,
                    remove_labels,
                    add_assignees,
                    remove_assignees,
                } => {
                    let add_labels: Vec<&str> = add_labels.iter().map(String::as_str).collect();
                    let remove_labels: Vec<&str> =
                        remove_labels.iter().map(String::as_str).collect();
                    let add_assignees: Vec<&str> =
                        add_assignees.iter().map(String::as_str).collect();
                    let remove_assignees: Vec<&str> =
                        remove_assignees.iter().map(String::as_str).collect();
                    let issue = self.update_issue(
                        reference,
                        title.as_deref(),
                        body.as_deref(),
                        state.as_ref(),
                        &add_labels,
                        &remove_labels,
                        &add_assignees,
                        &remove_assignees,
                    )?;
                    print_issue(&issue, cli.json);
                }

                IssueCommand::Close { reference } => {
                    let issue = self.update_issue(
                        reference,
                        None,
                        None,
                        Some(&IssueState::Closed),
                        &[],
                        &[],
                        &[],
                        &[],
                    )?;
                    print_issue(&issue, cli.json);
                }

                IssueCommand::Reopen { reference } => {
                    let issue = self.update_issue(
                        reference,
                        None,
                        None,
                        Some(&IssueState::Open),
                        &[],
                        &[],
                        &[],
                        &[],
                    )?;
                    print_issue(&issue, cli.json);
                }
            },
        }
        Ok(())
    }
}

fn print_issue(issue: &Issue, json: bool) {
    if json {
        println!(
            "{}",
            facet_json::to_string_pretty(issue).expect("serialize")
        );
        return;
    }
    let id = issue.display_id.as_deref().unwrap_or(&issue.oid[..8]);
    println!("issue #{id}");
    println!("title:     {}", issue.title);
    println!("state:     {}", issue.state.as_str());
    if !issue.labels.is_empty() {
        println!("labels:    {}", issue.labels.join(", "));
    }
    if !issue.assignees.is_empty() {
        println!("assignees: {}", issue.assignees.join(", "));
    }
    if !issue.body.is_empty() {
        println!();
        println!("{}", issue.body);
    }
}

fn print_issue_list(issues: &[Issue]) {
    for issue in issues {
        let id = issue.display_id.as_deref().unwrap_or(&issue.oid[..8]);
        println!("#{id:<10}  {}  [{}]", issue.title, issue.state.as_str());
    }
}
