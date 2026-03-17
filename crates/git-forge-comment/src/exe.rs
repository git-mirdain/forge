//! Execution logic for `git forge comment`.

use std::error::Error;
use std::process;

use git2::Repository;

use crate::cli::CommentCommand;
use crate::git2::{blob_oid_for_path, parse_ranges};
use crate::{Anchor, Comments, COMMENTS_REF_PREFIX, OBJECT_COMMENTS_REF};

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

fn open_editor_for_body(repo: &git2::Repository, initial: &str) -> Result<String, Box<dyn Error>> {
    use std::fs;
    use std::io::Write;

    let editor = resolve_editor(repo);
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

const FORGE_REFSPEC: &str = "+refs/forge/*:refs/forge/*";

fn fetch_forge_refs(repo: &git2::Repository) -> Result<(), Box<dyn Error>> {
    let mut remote = repo.find_remote("origin")?;
    remote.fetch(&[FORGE_REFSPEC], None, None)?;
    Ok(())
}

fn push_forge_ref(repo: &git2::Repository, ref_name: &str) -> Result<(), Box<dyn Error>> {
    let mut remote = repo.find_remote("origin")?;
    let refspec = format!("{ref_name}:{ref_name}");
    remote.push(&[&refspec], None)?;
    Ok(())
}

fn find_ref_for_comment(repo: &git2::Repository, oid: git2::Oid) -> Result<String, Box<dyn Error>> {
    let refs = repo.references_glob(&format!("{COMMENTS_REF_PREFIX}*"))?;
    for reference in refs {
        let reference = reference?;
        let ref_name = reference.name().ok_or("non-UTF-8 ref name")?.to_string();
        if repo.find_comment(&ref_name, oid)?.is_some() {
            return Ok(ref_name);
        }
    }
    Err(format!("comment {oid} not found in any forge ref").into())
}

fn default_target(repo: &git2::Repository, target: Option<String>) -> Result<String, Box<dyn Error>> {
    if let Some(t) = target { Ok(t) } else {
        let head = repo.head()?.peel_to_commit()?;
        Ok(format!("commit/{}", head.id()))
    }
}

/// Parsed target: ref name plus an optional anchor OID filter for object targets.
pub struct Target {
    /// The fully-qualified ref name (e.g. `refs/forge/comments/object`).
    pub ref_name: String,
    /// For object targets (`commit/<sha>`, `blob/<sha>`, `tree/<sha>`), the OID
    /// string from the target used to filter comments on the shared ref.
    pub anchor_filter: Option<String>,
}

/// Parse a user-facing target string into a ref name and optional anchor filter.
///
/// # Errors
///
/// Returns an error if the target string is not in `<kind>/<id>` form or uses an unknown kind.
pub fn parse_target(target: &str) -> Result<Target, Box<dyn Error>> {
    let Some((kind, id)) = target.split_once('/') else {
        return Err(format!(
            "invalid target {target:?}: expected \"<kind>/<id>\" \
             (e.g. \"issue/1\", \"commit/<sha>\")"
        )
        .into());
    };
    match kind {
        "commit" | "blob" | "tree" => Ok(Target {
            ref_name: OBJECT_COMMENTS_REF.to_string(),
            anchor_filter: Some(id.to_string()),
        }),
        "issue" | "review" => Ok(Target {
            ref_name: format!("{COMMENTS_REF_PREFIX}{target}"),
            anchor_filter: None,
        }),
        _ => Err(format!("unknown target kind {kind:?}").into()),
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

/// Extract the primary OID from an [`Anchor`].
#[must_use]
pub fn anchor_oid(anchor: &Anchor) -> git2::Oid {
    match anchor {
        Anchor::Blob { oid, .. } | Anchor::Commit(oid) | Anchor::Tree(oid) => *oid,
        Anchor::CommitRange { start, .. } => *start,
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
        anchor: Option<&str>,
        anchor_type: Option<&str>,
        range: Option<&str>,
    ) -> Result<git2::Oid, Box<dyn Error>> {
        let t = parse_target(target)?;
        let repo = self.repo();

        let anchor_obj = build_anchor(repo, anchor, anchor_type, range)?;
        let oid = repo.add_comment(&t.ref_name, &anchor_obj, body)?;
        Ok(oid)
    }

    pub fn reply_to_comment(
        &self,
        comment_oid_str: &str,
        body: &str,
    ) -> Result<(git2::Oid, String), Box<dyn Error>> {
        let repo = self.repo();
        let parent_oid = git2::Oid::from_str(comment_oid_str)
            .map_err(|e| format!("invalid comment OID {comment_oid_str:?}: {e}"))?;
        let ref_name = find_ref_for_comment(repo, parent_oid)?;
        let oid = repo.reply_to_comment(&ref_name, parent_oid, body)?;
        Ok((oid, ref_name))
    }

    pub fn edit_comment(
        &self,
        target: &str,
        comment_oid_str: &str,
        new_body: &str,
    ) -> Result<git2::Oid, Box<dyn Error>> {
        let t = parse_target(target)?;
        let repo = self.repo();
        let comment_oid = git2::Oid::from_str(comment_oid_str)
            .map_err(|e| format!("invalid comment OID {comment_oid_str:?}: {e}"))?;
        let oid = repo.edit_comment(&t.ref_name, comment_oid, new_body)?;
        Ok(oid)
    }

    pub fn resolve_comment(
        &self,
        comment_oid_str: &str,
        _message: Option<String>,
    ) -> Result<(git2::Oid, String), Box<dyn Error>> {
        let repo = self.repo();
        let comment_oid = git2::Oid::from_str(comment_oid_str)
            .map_err(|e| format!("invalid comment OID {comment_oid_str:?}: {e}"))?;
        let ref_name = find_ref_for_comment(repo, comment_oid)?;
        let oid = repo.resolve_comment(&ref_name, comment_oid)?;
        Ok((oid, ref_name))
    }

    pub fn list_comments(&self, target: &str) -> Result<(), Box<dyn Error>> {
        let t = parse_target(target)?;
        let repo = self.repo();
        let comments = repo.comments_on(&t.ref_name)?;
        for comment in &comments {
            if comment.resolved {
                continue;
            }
            if let Some(ref filter) = t.anchor_filter {
                let oid_hex = anchor_oid(&comment.anchor).to_string();
                if !oid_hex.starts_with(filter.as_str()) {
                    continue;
                }
            }
            let short_oid = &comment.oid.to_string()[..7];
            let first_line = comment.body.lines().next().unwrap_or("").trim();
            println!("{short_oid} {first_line}");
        }
        Ok(())
    }

    pub fn list_all_comments(&self) -> Result<(), Box<dyn Error>> {
        let repo = self.repo();
        let refs = repo.references_glob(&format!("{COMMENTS_REF_PREFIX}*"))?;
        for reference in refs {
            let reference = reference?;
            let ref_name = reference
                .name()
                .ok_or("non-UTF-8 ref name")?
                .to_string();
            let target_name = ref_name
                .strip_prefix(COMMENTS_REF_PREFIX)
                .unwrap_or(&ref_name);
            let comments = repo.comments_on(&ref_name)?;
            for comment in &comments {
                if comment.resolved {
                    continue;
                }
                let short_oid = &comment.oid.to_string()[..7];
                let first_line = comment.body.lines().next().unwrap_or("").trim();
                println!("{target_name} {short_oid} {first_line}");
            }
        }
        Ok(())
    }

    pub fn view_comment(&self, target: &str, comment_oid_str: &str) -> Result<(), Box<dyn Error>> {
        let t = parse_target(target)?;
        let ref_name = t.ref_name;
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
///
/// # Errors
///
/// Returns an error if the anchor or range is invalid.
pub fn build_anchor(
    repo: &git2::Repository,
    anchor: Option<&str>,
    anchor_type: Option<&str>,
    range: Option<&str>,
) -> Result<Anchor, Box<dyn Error>> {
    let anchor_str = anchor.unwrap_or("");

    if anchor_str.is_empty() {
        let head = repo.head()?.peel_to_commit()?;
        return Ok(Anchor::Commit(head.id()));
    }

    // Try to parse as OID first; fall back to path resolution.
    let (oid, inferred_blob) = if let Ok(oid) = git2::Oid::from_str(anchor_str) { (oid, false) } else {
        let oid = blob_oid_for_path(repo, anchor_str)
            .map_err(|e| format!("anchor {anchor_str:?} is not a valid OID or path: {e}"))?;
        (oid, true)
    };

    let line_ranges = range.map(parse_ranges).unwrap_or_default();

    if let Some(range_str) = range {
        let segment_count = range_str.split(',').filter(|s| !s.trim().is_empty()).count();
        if line_ranges.len() != segment_count {
            return Err(
                "malformed range: expected comma-separated \"start-end\" pairs (e.g. \"1-5,10-15\")"
                    .into(),
            );
        }
        for &(start, end) in &line_ranges {
            if start == 0 {
                return Err(format!("invalid range {start}-{end}: line numbers are 1-based").into());
            }
            if start > end {
                return Err(
                    format!("invalid range {start}-{end}: start must be <= end").into(),
                );
            }
        }
    }

    match anchor_type {
        Some("blob") | None if inferred_blob => {
            Ok(Anchor::Blob { oid, line_ranges })
        }
        Some("blob") => Ok(Anchor::Blob { oid, line_ranges }),
        Some("commit") => Ok(Anchor::Commit(oid)),
        Some("tree") => Ok(Anchor::Tree(oid)),
        Some("commit-range") => {
            let end_str = range.ok_or("commit-range requires --range <end-oid>")?;
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

fn run_inner(command: CommentCommand, push: bool, fetch: bool) -> Result<(), Box<dyn Error>> {
    let executor = Executor::from_env()?;
    let repo = executor.repo();

    match command {
        CommentCommand::New { target, body, anchor, anchor_type, range } => {
            let target = default_target(repo, target)?;
            let body = read_body(repo, body)?;
            if fetch {
                fetch_forge_refs(repo)?;
            }
            let oid = executor.new_comment(
                &target,
                &body,
                anchor.as_deref(),
                anchor_type.as_deref(),
                range.as_deref(),
            )?;
            if push {
                let t = parse_target(&target)?;
                push_forge_ref(repo, &t.ref_name)?;
            }
            println!("{oid}");
            let _ = std::fs::remove_file(repo.path().join("COMMENT_EDITMSG"));
        }

        CommentCommand::Reply { comment, body } => {
            let body = read_body(repo, body)?;
            if fetch {
                fetch_forge_refs(repo)?;
            }
            let (oid, ref_name) = executor.reply_to_comment(&comment, &body)?;
            if push {
                push_forge_ref(repo, &ref_name)?;
            }
            println!("{oid}");
            let _ = std::fs::remove_file(repo.path().join("COMMENT_EDITMSG"));
        }

        CommentCommand::Edit { target, comment, body } => {
            let target = default_target(repo, target)?;
            let t = parse_target(&target)?;
            let comment_oid = git2::Oid::from_str(&comment)
                .map_err(|e| format!("invalid comment OID {comment:?}: {e}"))?;
            let new_body = if let Some(b) = body { b } else {
                let existing = repo
                    .find_comment(&t.ref_name, comment_oid)?
                    .ok_or_else(|| format!("comment {comment} not found"))?;
                open_editor_for_body(repo, &existing.body)?
            };
            if fetch {
                fetch_forge_refs(repo)?;
            }
            let oid = executor.edit_comment(&target, &comment, &new_body)?;
            if push {
                push_forge_ref(repo, &t.ref_name)?;
            }
            println!("{oid}");
            let _ = std::fs::remove_file(repo.path().join("COMMENT_EDITMSG"));
        }

        CommentCommand::Resolve { comment, message } => {
            if fetch {
                fetch_forge_refs(repo)?;
            }
            let (oid, ref_name) = executor.resolve_comment(&comment, message)?;
            if push {
                push_forge_ref(repo, &ref_name)?;
            }
            println!("{oid}");
        }

        CommentCommand::List { target, all } => {
            if all {
                executor.list_all_comments()?;
            } else {
                let target = default_target(repo, target)?;
                executor.list_comments(&target)?;
            }
        }

        CommentCommand::View { target, comment } => {
            let target = default_target(repo, target)?;
            executor.view_comment(&target, &comment)?;
        }
    }

    Ok(())
}

/// Execute a `comment` subcommand.
pub fn run(command: CommentCommand, push: bool, fetch: bool) {
    if let Err(e) = run_inner(command, push, fetch) {
        eprintln!("Error: {e}");
        process::exit(1);
    }
}
