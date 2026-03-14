//! Execution logic for `git forge issue`.

use std::process;

use git2::Repository;

use crate::cli::{IssueCommand, StateArg};
use crate::issues::{IssueState, Issues};

fn open_repo() -> Repository {
    match Repository::open_from_env() {
        Ok(r) => r,
        Err(e) => {
            eprintln!("Error: {e}");
            process::exit(1);
        }
    }
}

fn run_inner(command: IssueCommand) -> Result<(), Box<dyn std::error::Error>> {
    let repo = open_repo();

    match command {
        IssueCommand::New {
            title,
            body,
            label,
            assignee,
        } => {
            let body = match body {
                Some(b) => b,
                None => {
                    use std::io::Read;
                    let mut buf = String::new();
                    std::io::stdin().read_to_string(&mut buf)?;
                    buf
                }
            };
            let id = repo.create_issue(&title, &body, &label, &assignee, None)?;
            eprintln!("Created issue #{id}: {title}");
        }

        IssueCommand::Edit {
            id,
            title,
            body,
            label,
            assignee,
            state,
        } => {
            let labels = if label.is_empty() {
                None
            } else {
                Some(label.as_slice())
            };
            let assignees = if assignee.is_empty() {
                None
            } else {
                Some(assignee.as_slice())
            };
            let issue_state = state.map(|s| match s {
                StateArg::Open => IssueState::Open,
                StateArg::Closed => IssueState::Closed,
            });
            repo.update_issue(
                id,
                title.as_deref(),
                body.as_deref(),
                labels,
                assignees,
                issue_state,
                None,
            )?;
            eprintln!("Updated issue #{id}.");
        }

        IssueCommand::List { state } => {
            let issue_state = match state {
                StateArg::Open => IssueState::Open,
                StateArg::Closed => IssueState::Closed,
            };
            let issues = repo.list_issues_by_state(issue_state, None)?;
            if issues.is_empty() {
                println!("No {} issues.", issue_state.as_str());
            } else {
                for issue in &issues {
                    println!(
                        "#{:>4}  [{}]  {}",
                        issue.id,
                        issue.meta.state.as_str(),
                        issue.meta.title,
                    );
                }
            }
        }

        IssueCommand::Status { id } => match repo.find_issue(id, None)? {
            None => {
                eprintln!("Issue #{id} not found.");
                process::exit(1);
            }
            Some(issue) => {
                println!(
                    "#{}: {} [{}]",
                    issue.id,
                    issue.meta.title,
                    issue.meta.state.as_str()
                );
            }
        },

        IssueCommand::Show { id } => match repo.find_issue(id, None)? {
            None => {
                eprintln!("Issue #{id} not found.");
                process::exit(1);
            }
            Some(issue) => {
                println!("Issue #{}", issue.id);
                println!("Title:  {}", issue.meta.title);
                println!("State:  {}", issue.meta.state.as_str());
                println!("Author: {}", issue.meta.author);
                if !issue.meta.labels.is_empty() {
                    println!("Labels: {}", issue.meta.labels.join(", "));
                }
                println!();
                println!("{}", issue.body);
                if !issue.comments.is_empty() {
                    println!();
                    println!("Comments ({})", issue.comments.len());
                    for (name, body) in &issue.comments {
                        println!("---");
                        println!("{name}");
                        println!("{body}");
                    }
                }
            }
        },
    }

    Ok(())
}

/// Execute an `issue` subcommand.
pub fn run(command: IssueCommand) {
    if let Err(e) = run_inner(command) {
        eprintln!("Error: {e}");
        process::exit(1);
    }
}
