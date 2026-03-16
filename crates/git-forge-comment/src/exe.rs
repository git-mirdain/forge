//! Execution logic for `git forge comment`.

use std::error::Error;
use std::process;

use git2::Repository;

use crate::cli::CommentCommand;
use crate::{Anchor, Comments, COMMENTS_REF_PREFIX};

fn parse_target(target: &str) -> Result<String, Box<dyn Error>> {
    if target.contains('/') {
        Ok(format!("{COMMENTS_REF_PREFIX}{target}"))
    } else {
        Err(format!("invalid target {target:?}: expected \"<kind>/<id>\" (e.g. \"issue/1\", \"commit/<sha>\", \"blob/<sha>\")").into())
    }
}

fn read_body(body: Option<String>) -> Result<String, Box<dyn Error>> {
    match body {
        Some(b) => Ok(b),
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
            Anchor::Blob { oid, line_range } => {
                if let Some((s, e)) = line_range {
                    println!("anchor: blob {oid} lines {s}-{e}");
                } else {
                    println!("anchor: blob {oid}");
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

fn build_anchor(
    repo: &git2::Repository,
    anchor: Option<String>,
    anchor_type: Option<String>,
    range: Option<String>,
) -> Result<Anchor, Box<dyn Error>> {
    let oid_str = anchor.as_deref().unwrap_or("");
    let kind = anchor_type.as_deref().unwrap_or("commit");

    if oid_str.is_empty() {
        // Default: anchor to HEAD commit
        let head = repo.head()?.peel_to_commit()?;
        return Ok(Anchor::Commit(head.id()));
    }

    let oid = git2::Oid::from_str(oid_str)
        .map_err(|e| format!("invalid anchor OID {oid_str:?}: {e}"))?;

    match kind {
        "blob" => {
            let line_range = range.as_deref().and_then(|r| {
                let (s, e) = r.split_once('-')?;
                Some((s.parse::<u32>().ok()?, e.parse::<u32>().ok()?))
            });
            Ok(Anchor::Blob { oid, line_range })
        }
        "commit" => Ok(Anchor::Commit(oid)),
        "tree" => Ok(Anchor::Tree(oid)),
        "commit-range" => {
            let end_str = range.as_deref().ok_or("commit-range requires --range <start>-<end>")?;
            let end = git2::Oid::from_str(end_str)
                .map_err(|e| format!("invalid range end OID: {e}"))?;
            Ok(Anchor::CommitRange { start: oid, end })
        }
        other => Err(format!("unknown anchor-type: {other:?}").into()),
    }
}

fn run_inner(command: CommentCommand) -> Result<(), Box<dyn Error>> {
    let executor = Executor::from_env()?;

    match command {
        CommentCommand::New { target, body, anchor, anchor_type, range } => {
            let body = read_body(body)?;
            let oid = executor.new_comment(&target, &body, anchor, anchor_type, range)?;
            println!("{oid}");
        }

        CommentCommand::Reply { target, comment, body } => {
            let body = read_body(body)?;
            let oid = executor.reply_to_comment(&target, &comment, &body)?;
            println!("{oid}");
        }

        CommentCommand::Resolve { target, comment, message } => {
            let oid = executor.resolve_comment(&target, &comment, message)?;
            println!("{oid}");
        }

        CommentCommand::List { target } => {
            executor.list_comments(&target)?;
        }

        CommentCommand::View { target, comment } => {
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
