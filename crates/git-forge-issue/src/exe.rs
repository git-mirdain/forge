//! Execution logic for `git forge issue`.

use std::process;

use git2::Repository;

use crate::cli::{IssueCommand, StateArg};
use crate::{IssueState, Issues};

/// Resolve the editor to use, matching Git's own precedence:
/// `GIT_EDITOR` → `core.editor` (git config) → `VISUAL` → `EDITOR` → `"vi"`.
fn resolve_editor(repo: &git2::Repository) -> Result<String, Box<dyn std::error::Error>> {
    if let Ok(val) = std::env::var("GIT_EDITOR") {
        if !val.is_empty() {
            return Ok(val);
        }
    }
    if let Ok(cfg) = repo.config() {
        if let Ok(val) = cfg.get_string("core.editor") {
            if !val.is_empty() {
                return Ok(val);
            }
        }
    }
    for var in &["VISUAL", "EDITOR"] {
        if let Ok(val) = std::env::var(var) {
            if !val.is_empty() {
                return Ok(val);
            }
        }
    }
    Ok("vi".to_string())
}

/// Parse issue template with TOML frontmatter.
/// Returns (title, body).
fn parse_issue_template(content: &str) -> Result<(String, String), Box<dyn std::error::Error>> {
    // Check if content starts with +++
    if !content.starts_with("+++\n") {
        return Err("Template must start with +++".into());
    }

    // Find the closing +++
    let rest = &content[4..];
    let closing_pos = match rest.find("\n+++\n") {
        Some(pos) => pos,
        None => return Err("Could not find closing +++".into()),
    };

    let frontmatter = &rest[..closing_pos];
    let body_start = closing_pos + 5; // length of "\n+++\n"
    let body = rest[body_start..].trim_end().to_string();

    // Parse title from frontmatter
    let title = frontmatter
        .lines()
        .find_map(|line| {
            if line.starts_with("title = ") {
                let title_str = &line[8..];
                // Remove quotes
                if (title_str.starts_with('"') && title_str.ends_with('"'))
                    || (title_str.starts_with('\'') && title_str.ends_with('\''))
                {
                    Some(title_str[1..title_str.len() - 1].to_string())
                } else {
                    Some(title_str.to_string())
                }
            } else {
                None
            }
        })
        .ok_or("Could not find title in frontmatter")?;

    Ok((title, body))
}

#[allow(dead_code)]
fn open_repo() -> Repository {
    match Repository::open_from_env() {
        Ok(r) => r,
        Err(e) => {
            eprintln!("Error: {e}");
            process::exit(1);
        }
    }
}

/// A wrapper type which manipulates issues for the provided repository.
struct Executor(git2::Repository);

impl Executor {
    /// Constructs an `Executor` from a path to a repository.
    #[allow(dead_code)]
    pub fn from_path(path: &str) -> Result<Self, git2::Error> {
        let repo = Repository::open(path)?;
        Ok(Self(repo))
    }

    /// Constructs an `Executor` from [`Repository::open_from_env()`].
    pub fn from_env() -> Result<Self, git2::Error> {
        let repo = Repository::open_from_env()?;
        Ok(Self(repo))
    }

    /// Return a reference the underlying [`git2::Repository`].
    pub fn repo(&self) -> &git2::Repository {
        &self.0
    }

    /// Lists issues for the repository, optionally filtered by state.
    #[allow(dead_code)]
    pub fn list_issues(&self, state: Option<IssueState>) -> Result<(), Box<dyn std::error::Error>> {
        let repo = self.repo();

        let issues = match state {
            Some(state) => repo.list_issues_by_state(state, None)?,
            None => repo.list_issues(None)?,
        };

        for issue in issues {
            println!("#{}\t{}", issue.id, issue.meta.title);
        }
        Ok(())
    }

    /// Creates a new issue with the given title and body.
    pub fn create_issue(
        &self,
        title: Option<&str>,
        body: Option<&str>,
        label: Option<&[String]>,
        assignee: Option<&[String]>,
    ) -> Result<u64, Box<dyn std::error::Error>> {
        let repo = self.repo();
        let labels = label.unwrap_or_default();
        let assignees = assignee.unwrap_or_default();
        let title = title.ok_or("Title is required")?;
        let body = body.ok_or("Body is required")?;
        let id = repo.create_issue(title, body, labels, assignees, None)?;
        Ok(id)
    }

    /// Creates a new issue interactively using an editor.
    pub fn create_issue_interactive(&self) -> Result<u64, Box<dyn std::error::Error>> {
        use std::fs;
        use std::io::Write;
        use std::process::Command;

        let repo = self.repo();
        let editor = resolve_editor(repo)?;

        let edit_path = repo.path().join("ISSUE_EDITMSG");
        let template = "+++\ntitle = \"\"\n+++\n\n";
        {
            let mut f = fs::File::create(&edit_path)?;
            f.write_all(template.as_bytes())?;
        }

        // Open editor
        let status = Command::new(&editor).arg(&edit_path).status()?;

        if !status.success() {
            return Err("Editor exited with error".into());
        }

        // Read and parse the file
        let content = fs::read_to_string(&edit_path)?;
        let (title, body) = parse_issue_template(&content)?;

        if title.trim().is_empty() {
            return Err("Title cannot be empty".into());
        }

        let id = repo.create_issue(&title, &body, &[], &[], None)?;
        Ok(id)
    }

    /// Updates an existing issue.
    pub fn edit_issue(
        &self,
        id: u64,
        title: Option<&str>,
        body: Option<&str>,
        labels: Option<&[String]>,
        assignees: Option<&[String]>,
        state: Option<IssueState>,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let repo = self.repo();
        repo.update_issue(id, title, body, labels, assignees, state, None)?;
        Ok(())
    }

    /// Displays the full details of an issue.
    pub fn show_issue(&self, id: u64) -> Result<(), Box<dyn std::error::Error>> {
        let repo = self.repo();
        match repo.find_issue(id, None)? {
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
        }
        Ok(())
    }

    /// Displays the status of an issue.
    pub fn status_issue(&self, id: u64) -> Result<(), Box<dyn std::error::Error>> {
        let repo = self.repo();
        match repo.find_issue(id, None)? {
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
        }
        Ok(())
    }
}

fn run_inner(command: IssueCommand) -> Result<(), Box<dyn std::error::Error>> {
    let executor = Executor::from_env()?;

    match command {
        IssueCommand::New {
            title,
            body,
            label,
            assignee,
            interactive,
        } => {
            if interactive {
                let id = executor.create_issue_interactive()?;
                eprintln!("Created issue #{id}");
            } else {
                let title = title
                    .as_deref()
                    .ok_or("Title is required (or use --interactive)")?;
                let body = if let Some(b) = body {
                    b
                } else {
                    use std::io::Read;
                    let mut buf = String::new();
                    std::io::stdin().read_to_string(&mut buf)?;
                    buf
                };
                let id = executor.create_issue(
                    Some(title),
                    Some(&body),
                    Some(&label),
                    Some(&assignee),
                )?;
                eprintln!("Created issue #{id}: {title}");
            }
        }

        IssueCommand::Edit {
            id,
            title,
            body,
            label,
            assignee,
            state,
        } => {
            // Check if any specific fields are provided
            let has_fields = title.is_some()
                || body.is_some()
                || !label.is_empty()
                || !assignee.is_empty()
                || state.is_some();

            // Default to interactive when no fields provided
            if !has_fields {
                use std::fs;
                use std::io::Write;
                use std::process::Command;

                let repo = executor.repo();
                let editor = resolve_editor(repo)?;

                // Fetch the current issue
                let issue = repo
                    .find_issue(id, None)?
                    .ok_or(format!("Issue #{id} not found"))?;

                let edit_path = repo.path().join("ISSUE_EDITMSG");
                let template = format!(
                    "+++\ntitle = \"{}\"\n+++\n\n{}",
                    issue.meta.title.replace('"', "\\\""),
                    issue.body
                );
                {
                    let mut f = fs::File::create(&edit_path)?;
                    f.write_all(template.as_bytes())?;
                }

                // Open editor
                let status = Command::new(&editor).arg(&edit_path).status()?;

                if !status.success() {
                    return Err("Editor exited with error".into());
                }

                // Read and parse the file
                let content = fs::read_to_string(&edit_path)?;
                let (title, body) = parse_issue_template(&content)?;

                if title.trim().is_empty() {
                    return Err("Title cannot be empty".into());
                }

                repo.update_issue(id, Some(&title), Some(&body), None, None, None, None)?;
                eprintln!("Updated issue #{id}.");
            } else {
                let labels = if label.is_empty() { None } else { Some(label) };
                let assignees = if assignee.is_empty() {
                    None
                } else {
                    Some(assignee)
                };
                let issue_state = state.map(|s| match s {
                    StateArg::Open => IssueState::Open,
                    StateArg::Closed => IssueState::Closed,
                });
                executor.edit_issue(
                    id,
                    title.as_deref(),
                    body.as_deref(),
                    labels.as_deref(),
                    assignees.as_deref(),
                    issue_state,
                )?;
                eprintln!("Updated issue #{id}.");
            }
        }

        IssueCommand::List { state } => {
            let issue_state = match state {
                StateArg::Open => IssueState::Open,
                StateArg::Closed => IssueState::Closed,
            };
            let repo = executor.repo();
            let issues = repo.list_issues_by_state(issue_state, None)?;
            if issues.is_empty() {
                println!("No {} issues.", issue_state.as_str());
            } else {
                for issue in &issues {
                    println!(
                        "#{} [{}] {}",
                        issue.id,
                        issue.meta.state.as_str(),
                        issue.meta.title,
                    );
                }
            }
        }

        IssueCommand::Status { id } => {
            executor.status_issue(id)?;
        }

        IssueCommand::Show { id } => {
            executor.show_issue(id)?;
        }
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
