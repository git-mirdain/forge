//! Execution logic for `git forge issue`.

use std::process;

use git2::Repository;

use crate::cli::{IssueCommand, StateArg};
use crate::{IssueState, Issues};

/// Resolve the editor to use, matching Git's own precedence:
/// `GIT_EDITOR` → `core.editor` (git config) → `VISUAL` → `EDITOR` → `"vi"`.
fn resolve_editor(repo: &git2::Repository) -> String {
    if let Ok(val) = std::env::var("GIT_EDITOR")
        && !val.is_empty() {
            return val;
        }
    if let Ok(cfg) = repo.config()
        && let Ok(val) = cfg.get_string("core.editor")
            && !val.is_empty() {
                return val;
            }
    for var in &["VISUAL", "EDITOR"] {
        if let Ok(val) = std::env::var(var)
            && !val.is_empty() {
                return val;
            }
    }
    "vi".to_string()
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
    let Some(closing_pos) = rest.find("\n+++\n") else { return Err("Could not find closing +++".into()) };

    let frontmatter = &rest[..closing_pos];
    let body_start = closing_pos + 5; // length of "\n+++\n"
    let body = rest[body_start..].trim_end().to_string();

    // Parse title from frontmatter
    let title = frontmatter
        .lines()
        .find_map(|line| {
            if let Some(title_str) = line.strip_prefix("title = ") {
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

const FORGE_REFSPEC: &str = "+refs/forge/*:refs/forge/*";
const MAX_PUSH_ATTEMPTS: usize = 3;

// TODO audit: credential_callbacks uses global git config, not repo config
fn fetch_forge_refs(repo: &git2::Repository) -> Result<(), Box<dyn std::error::Error>> {
    let mut remote = repo.find_remote("origin")?;
    let mut fetch_opts = git_forge_core::credentials::fetch_options()?;
    remote.fetch(&[FORGE_REFSPEC], Some(&mut fetch_opts), None)?;
    Ok(())
}

// TODO audit: credential_callbacks uses global git config, not repo config
fn push_forge_ref(
    repo: &git2::Repository,
    ref_name: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut remote = repo.find_remote("origin")?;
    let refspec = format!("{ref_name}:{ref_name}");
    let mut push_opts = git_forge_core::credentials::push_options()?;
    remote.push(&[&refspec], Some(&mut push_opts))?;
    Ok(())
}

fn read_issue_from_editor(
    repo: &git2::Repository,
) -> Result<(String, String), Box<dyn std::error::Error>> {
    use std::fs;
    use std::io::Write;
    use std::process::Command;

    let editor = resolve_editor(repo);
    let edit_path = repo.path().join("ISSUE_EDITMSG");
    let template = "+++\ntitle = \"\"\n+++\n\n";
    {
        let mut f = fs::File::create(&edit_path)?;
        f.write_all(template.as_bytes())?;
    }

    let status = Command::new(&editor).arg(&edit_path).status()?;
    if !status.success() {
        return Err("Editor exited with error".into());
    }

    let content = fs::read_to_string(&edit_path)?;
    let (title, body) = parse_issue_template(&content)?;
    if title.trim().is_empty() {
        return Err("Title cannot be empty".into());
    }
    Ok((title, body))
}

fn create_and_push_issue(
    repo: &git2::Repository,
    title: &str,
    body: &str,
    labels: &[String],
    assignees: &[String],
    fetch: bool,
) -> Result<u64, Box<dyn std::error::Error>> {
    let mut prev_id = None;
    for attempt in 0..MAX_PUSH_ATTEMPTS {
        if fetch || attempt > 0 {
            fetch_forge_refs(repo)?;
        }
        if let Some(old_id) = prev_id {
            let old_ref = format!("{}{old_id}", crate::ISSUES_REF_PREFIX);
            let _ = repo.find_reference(&old_ref).map(|mut r| r.delete());
        }
        let id = repo.create_issue(title, body, labels, assignees, None)?;
        let ref_name = format!("{}{id}", crate::ISSUES_REF_PREFIX);
        match push_forge_ref(repo, &ref_name) {
            Ok(()) => return Ok(id),
            Err(e) => {
                eprintln!("Push rejected for issue #{id}: {e}; retrying...");
                prev_id = Some(id);
            }
        }
    }
    Err("push rejected after multiple retries; try again".into())
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

#[allow(clippy::too_many_lines)]
fn run_inner(command: IssueCommand, push: bool, fetch: bool) -> Result<(), Box<dyn std::error::Error>> {
    let executor = Executor::from_env()?;

    match command {
        IssueCommand::New { title, body, label, assignee } => {
            use std::io::IsTerminal;

            let (title, body, labels, assignees) =
                if title.is_none() && std::io::stdin().is_terminal() {
                    let (t, b) = read_issue_from_editor(executor.repo())?;
                    (t, b, vec![], vec![])
                } else {
                    let t = title.ok_or("Title is required")?;
                    let b = if let Some(b) = body {
                        b
                    } else {
                        use std::io::Read;
                        let mut buf = String::new();
                        std::io::stdin().read_to_string(&mut buf)?;
                        buf
                    };
                    (t, b, label, assignee)
                };

            let repo = executor.repo();
            let id = if push {
                create_and_push_issue(repo, &title, &body, &labels, &assignees, fetch)?
            } else {
                repo.create_issue(&title, &body, &labels, &assignees, None)?
            };
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
            // Check if any specific fields are provided
            let has_fields = title.is_some()
                || body.is_some()
                || !label.is_empty()
                || !assignee.is_empty()
                || state.is_some();

            let repo = executor.repo();
            if fetch {
                fetch_forge_refs(repo)?;
            }

            // Default to interactive when no fields provided
            if has_fields {
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
                if push {
                    let ref_name = format!("{}{id}", crate::ISSUES_REF_PREFIX);
                    push_forge_ref(repo, &ref_name)?;
                }
                eprintln!("Updated issue #{id}.");
            } else {
                use std::fs;
                use std::io::Write;
                use std::process::Command;

                let editor = resolve_editor(repo);

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
                if push {
                    let ref_name = format!("{}{id}", crate::ISSUES_REF_PREFIX);
                    push_forge_ref(repo, &ref_name)?;
                }
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
pub fn run(command: IssueCommand, push: bool, fetch: bool) {
    if let Err(e) = run_inner(command, push, fetch) {
        eprintln!("Error: {e}");
        process::exit(1);
    }
}
