//! Execution logic for `git forge comment`.

use std::error::Error;
use std::process;

use git2::Repository;

use crate::cli::CommentCommand;
use crate::git2::{blob_oid_for_path, parse_ranges};
use crate::{Anchor, Comments, COMMENTS_REF_PREFIX};

/// Resolve the editor to use, matching Git's own precedence:
/// `GIT_EDITOR` → `core.editor` (git config) → `VISUAL` → `EDITOR` → `"vi"`.
fn resolve_editor(repo: &git2::Repository) -> Result<String, Box<dyn Error>> {
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

fn open_editor_for_body(repo: &git2::Repository, initial: &str) -> Result<String, Box<dyn Error>> {
    use std::fs;
    use std::io::Write;

    let editor = resolve_editor(repo)?;
    let edit_path = repo.path().join("COMMENT_EDITMSG");
    {
        let mut f = fs::File::create(&edit_path)?;
        f.write_all(initial.as_bytes())?;
    }
    let status = std::process::Command::new(&editor).arg(&edit_path).status()?;
    if !status.success() {
        return Err("Editor exited with error".into());
    }
    let body = fs::read_to_string(&edit_path)?;
    Ok(body)
}

fn default_target(repo: &git2::Repository, target: Option<String>) -> Result<String, Box<dyn Error>> {
    match target {
        Some(t) => Ok(t),
        None => {
            let head = repo.head()?.peel_to_commit()?;
            Ok(format!("commit/{}", head.id()))
        }
    }
}

fn parse_target(target: &str) -> Result<String, Box<dyn Error>> {
    if target.contains('/') {
        Ok(format!("{COMMENTS_REF_PREFIX}{target}"))
    } else {
        Err(format!("invalid target {target:?}: expected \"<kind>/<id>\" (e.g. \"issue/1\", \"commit/<sha>\", \"blob/<sha>\")").into())
    }
}

fn read_body(repo: &git2::Repository, body: Option<String>) -> Result<String, Box<dyn Error>> {
    use std::io::IsTerminal;
    match body {
        Some(b) => Ok(b),
        None if std::io::stdin().is_terminal() => open_editor_for_body(repo, ""),
        None => {
            use std::io::Read;
            let mut buf = String::new();
            std::io::stdin().read_to_string(&mut buf)?;
            Ok(buf)
        }
    }
}

struct Executor(git2::Repository);

impl Executor {
    pub fn from_env() -> Result<Self, git2::Error> {
        let repo = Repository::open_from_env()?;
        Ok(Self(repo))
    }

    pub fn repo(&self) -> &git2::Repository {
        &self.0
    }

    pub fn new_comment(
        &self,
        target: &str,
        body: &str,
        anchor: Option<String>,
        anchor_type: Option<String>,
        range: Option<String>,
    ) -> Result<git2::Oid, Box<dyn Error>> {
        let ref_name = parse_target(target)?;
        let repo = self.repo();

        let anchor_obj = build_anchor(repo, anchor, anchor_type, range)?;
        let oid = repo.add_comment(&ref_name, &anchor_obj, body)?;
        Ok(oid)
    }

    pub fn reply_to_comment(
        &self,
        target: &str,
        comment_oid_str: &str,
        body: &str,
    ) -> Result<git2::Oid, Box<dyn Error>> {
        let ref_name = parse_target(target)?;
        let repo = self.repo();
        let parent_oid = git2::Oid::from_str(comment_oid_str)
            .map_err(|e| format!("invalid comment OID {comment_oid_str:?}: {e}"))?;
        let oid = repo.reply_to_comment(&ref_name, parent_oid, body)?;
        Ok(oid)
    }

    pub fn edit_comment(
        &self,
        target: &str,
        comment_oid_str: &str,
        new_body: &str,
    ) -> Result<git2::Oid, Box<dyn Error>> {
        let ref_name = parse_target(target)?;
        let repo = self.repo();
        let comment_oid = git2::Oid::from_str(comment_oid_str)
            .map_err(|e| format!("invalid comment OID {comment_oid_str:?}: {e}"))?;
        let oid = repo.edit_comment(&ref_name, comment_oid, new_body)?;
        Ok(oid)
    }

    pub fn resolve_comment(
        &self,
        target: &str,
        comment_oid_str: &str,
        _message: Option<String>,
    ) -> Result<git2::Oid, Box<dyn Error>> {
        let ref_name = parse_target(target)?;
        let repo = self.repo();
        let comment_oid = git2::Oid::from_str(comment_oid_str)
            .map_err(|e| format!("invalid comment OID {comment_oid_str:?}: {e}"))?;
        let oid = repo.resolve_comment(&ref_name, comment_oid)?;
        Ok(oid)
    }

    pub fn list_comments(&self, target: &str) -> Result<(), Box<dyn Error>> {
        let ref_name = parse_target(target)?;
        let repo = self.repo();
        let comments = repo.comments_on(&ref_name)?;
        for comment in &comments {
            let short_oid = &comment.oid.to_string()[..7];
            let resolved = if comment.resolved { " [resolved]" } else { "" };
            let first_line = comment.body.lines().next().unwrap_or("").trim();
            println!("{short_oid}{resolved} {first_line}");
        }
        Ok(())
    }

    pub fn view_comment(&self, target: &str, comment_oid_str: &str) -> Result<(), Box<dyn Error>> {
        let ref_name = parse_target(target)?;
        let repo = self.repo();
        let oid = git2::Oid::from_str(comment_oid_str)
            .map_err(|e| format!("invalid comment OID {comment_oid_str:?}: {e}"))?;
        let comment = repo
            .find_comment(&ref_name, oid)?
            .ok_or_else(|| format!("comment {comment_oid_str} not found"))?;

        println!("commit {}", comment.oid);
        match &comment.anchor {
            Anchor::Blob { oid, line_ranges } => {
                if line_ranges.is_empty() {
                    println!("anchor: blob {oid}");
                } else {
                    let ranges = line_ranges
                        .iter()
                        .map(|(s, e)| format!("{s}-{e}"))
                        .collect::<Vec<_>>()
                        .join(",");
                    println!("anchor: blob {oid} lines {ranges}");
                }
            }
            Anchor::Commit(oid) => println!("anchor: commit {oid}"),
            Anchor::Tree(oid) => println!("anchor: tree {oid}"),
            Anchor::CommitRange { start, end } => println!("anchor: commits {start}..{end}"),
        }
        if let Some(p) = comment.parent_oid {
            println!("parent: {p}");
        }
        if comment.resolved {
            println!("resolved: true");
        }
        println!();
        print!("{}", comment.body);
        if !comment.body.ends_with('\n') {
            println!();
        }
        Ok(())
    }
}

/// Resolve an anchor argument to an [`Anchor`].
///
/// `anchor` may be an OID or a file path (resolved against HEAD's tree).
/// When `anchor_type` is omitted the object's kind is used to infer the type.
/// `range` is a comma-separated list of `start-end` pairs for blob anchors.
pub fn build_anchor(
    repo: &git2::Repository,
    anchor: Option<String>,
    anchor_type: Option<String>,
    range: Option<String>,
) -> Result<Anchor, Box<dyn Error>> {
    let anchor_str = anchor.as_deref().unwrap_or("");

    if anchor_str.is_empty() {
        let head = repo.head()?.peel_to_commit()?;
        return Ok(Anchor::Commit(head.id()));
    }

    // Try to parse as OID first; fall back to path resolution.
    let (oid, inferred_blob) = match git2::Oid::from_str(anchor_str) {
        Ok(oid) => (oid, false),
        Err(_) => {
            let oid = blob_oid_for_path(repo, anchor_str)
                .map_err(|e| format!("anchor {anchor_str:?} is not a valid OID or path: {e}"))?;
            (oid, true)
        }
    };

    let line_ranges = range.as_deref().map(parse_ranges).unwrap_or_default();

    match anchor_type.as_deref() {
        Some("blob") | None if inferred_blob => {
            Ok(Anchor::Blob { oid, line_ranges })
        }
        Some("blob") => Ok(Anchor::Blob { oid, line_ranges }),
        Some("commit") => Ok(Anchor::Commit(oid)),
        Some("tree") => Ok(Anchor::Tree(oid)),
        Some("commit-range") => {
            let end_str = range.as_deref().ok_or("commit-range requires --range <end-oid>")?;
            let end = git2::Oid::from_str(end_str)
                .map_err(|e| format!("invalid range end OID: {e}"))?;
            Ok(Anchor::CommitRange { start: oid, end })
        }
        // No explicit type and not a path — infer from object kind.
        None => {
            match repo.find_object(oid, None)?.kind() {
                Some(git2::ObjectType::Blob) => Ok(Anchor::Blob { oid, line_ranges }),
                Some(git2::ObjectType::Tree) => Ok(Anchor::Tree(oid)),
                _ => Ok(Anchor::Commit(oid)),
            }
        }
        Some(other) => Err(format!("unknown anchor-type: {other:?}").into()),
    }
}

fn run_inner(command: CommentCommand) -> Result<(), Box<dyn Error>> {
    let executor = Executor::from_env()?;

    match command {
        CommentCommand::New { target, body, anchor, anchor_type, range } => {
            let target = default_target(executor.repo(), target)?;
            let body = read_body(executor.repo(), body)?;
            let oid = executor.new_comment(&target, &body, anchor, anchor_type, range)?;
            println!("{oid}");
        }

        CommentCommand::Reply { target, comment, body } => {
            let target = default_target(executor.repo(), target)?;
            let body = read_body(executor.repo(), body)?;
            let oid = executor.reply_to_comment(&target, &comment, &body)?;
            println!("{oid}");
        }

        CommentCommand::Edit { target, comment, body } => {
            let target = default_target(executor.repo(), target)?;
            let repo = executor.repo();
            let ref_name = parse_target(&target)?;
            let comment_oid = git2::Oid::from_str(&comment)
                .map_err(|e| format!("invalid comment OID {comment:?}: {e}"))?;
            let new_body = match body {
                Some(b) => b,
                None => {
                    let existing = repo
                        .find_comment(&ref_name, comment_oid)?
                        .ok_or_else(|| format!("comment {comment} not found"))?;
                    open_editor_for_body(repo, &existing.body)?
                }
            };
            let oid = executor.edit_comment(&target, &comment, &new_body)?;
            println!("{oid}");
        }

        CommentCommand::Resolve { target, comment, message } => {
            let target = default_target(executor.repo(), target)?;
            let oid = executor.resolve_comment(&target, &comment, message)?;
            println!("{oid}");
        }

        CommentCommand::List { target } => {
            let target = default_target(executor.repo(), target)?;
            executor.list_comments(&target)?;
        }

        CommentCommand::View { target, comment } => {
            let target = default_target(executor.repo(), target)?;
            executor.view_comment(&target, &comment)?;
        }
    }

    Ok(())
}

/// Execute a `comment` subcommand.
pub fn run(command: CommentCommand) {
    if let Err(e) = run_inner(command) {
        eprintln!("Error: {e}");
        process::exit(1);
    }
}
