//! `git2::Repository` implementation of [`Comments`].

use std::fmt::Write as _;

use git2::Repository;

use crate::{Anchor, Comment, Comments};

const EMPTY_TREE_OID: &str = "4b825dc642cb6eb9a060e54bf8d69288fbee4904";

struct Trailers {
    anchor_oid: Option<git2::Oid>,
    anchor_ranges: Vec<(u32, u32)>,
    anchor_end: Option<git2::Oid>,
    resolved: bool,
    replaces_oid: Option<git2::Oid>,
}

/// Parse a comma-separated list of `start-end` pairs (e.g. `"1-5,10-15"`).
pub(crate) fn parse_ranges(s: &str) -> Vec<(u32, u32)> {
    s.split(',')
        .filter_map(|part| {
            let (start, end) = part.trim().split_once('-')?;
            Some((start.parse::<u32>().ok()?, end.parse::<u32>().ok()?))
        })
        .collect()
}

/// Serialize a slice of ranges to `"start-end,start-end"` form.
fn format_ranges(ranges: &[(u32, u32)]) -> String {
    ranges.iter().map(|(s, e)| format!("{s}-{e}")).collect::<Vec<_>>().join(",")
}

fn parse_trailers(msg: &str) -> Trailers {
    let mut t = Trailers {
        anchor_oid: None,
        anchor_ranges: Vec::new(),
        anchor_end: None,
        resolved: false,
        replaces_oid: None,
    };
    for line in msg.lines() {
        if let Some(v) = line.strip_prefix("Anchor: ") {
            t.anchor_oid = git2::Oid::from_str(v.trim()).ok();
        } else if let Some(v) = line.strip_prefix("Anchor-Range: ") {
            t.anchor_ranges = parse_ranges(v.trim());
        } else if let Some(v) = line.strip_prefix("Anchor-End: ") {
            t.anchor_end = git2::Oid::from_str(v.trim()).ok();
        } else if let Some(v) = line.strip_prefix("Resolved: ") {
            t.resolved = v.trim() == "true";
        } else if let Some(v) = line.strip_prefix("Replaces: ") {
            t.replaces_oid = git2::Oid::from_str(v.trim()).ok();
        }
    }
    t
}

fn body_from_message(msg: &str) -> String {
    let is_trailer = |line: &str| -> bool {
        line.find(": ")
            .is_some_and(|pos| {
                let key = &line[..pos];
                !key.is_empty() && key.chars().all(|c| c.is_alphanumeric() || c == '-')
            })
    };

    if let Some(split) = msg.rfind("\n\n") {
        let after = &msg[split + 2..];
        if after.lines().all(|l| l.is_empty() || is_trailer(l)) {
            return msg[..split].trim_end().to_string();
        }
    }
    msg.trim_end().to_string()
}

fn anchor_from_trailers(repo: &Repository, t: &Trailers) -> Result<Anchor, git2::Error> {
    let oid = t.anchor_oid.ok_or_else(|| git2::Error::from_str("missing Anchor trailer"))?;

    if let Some(end) = t.anchor_end {
        return Ok(Anchor::CommitRange { start: oid, end });
    }

    match repo.find_object(oid, None)?.kind() {
        Some(git2::ObjectType::Blob) => {
            Ok(Anchor::Blob { oid, line_ranges: t.anchor_ranges.clone() })
        }
        Some(git2::ObjectType::Tree) => Ok(Anchor::Tree(oid)),
        _ => Ok(Anchor::Commit(oid)),
    }
}

fn comment_from_commit(repo: &Repository, commit: &git2::Commit<'_>) -> Result<Comment, git2::Error> {
    let msg = commit.message().unwrap_or("");
    let trailers = parse_trailers(msg);
    let body = body_from_message(msg);
    let anchor = anchor_from_trailers(repo, &trailers)?;

    let parent_oid = if commit.parent_count() >= 2 { Some(commit.parent_id(1)?) } else { None };

    Ok(Comment {
        oid: commit.id(),
        anchor,
        body,
        resolved: trailers.resolved,
        parent_oid,
        replaces_oid: trailers.replaces_oid,
    })
}

fn build_message(
    body: &str,
    anchor: &Anchor,
    resolved: bool,
    replaces: Option<git2::Oid>,
) -> String {
    let mut msg = body.trim_end().to_string();
    msg.push_str("\n\n");

    match anchor {
        Anchor::Blob { oid, line_ranges } => {
            writeln!(msg, "Anchor: {oid}").expect("writing to a String is infallible");
            if !line_ranges.is_empty() {
                writeln!(msg, "Anchor-Range: {}", format_ranges(line_ranges)).expect("writing to a String is infallible");
            }
        }
        Anchor::Commit(oid) | Anchor::Tree(oid) => writeln!(msg, "Anchor: {oid}").expect("writing to a String is infallible"),
        Anchor::CommitRange { start, end } => {
            writeln!(msg, "Anchor: {start}").expect("writing to a String is infallible");
            writeln!(msg, "Anchor-End: {end}").expect("writing to a String is infallible");
        }
    }

    if resolved {
        msg.push_str("Resolved: true\n");
    }

    if let Some(oid) = replaces {
        writeln!(msg, "Replaces: {oid}").expect("writing to a String is infallible");
    }

    msg
}

fn current_tip<'a>(repo: &'a Repository, ref_name: &str) -> Result<Option<git2::Commit<'a>>, git2::Error> {
    match repo.find_reference(ref_name) {
        Ok(r) => Ok(Some(r.peel_to_commit()?)),
        Err(e) if e.code() == git2::ErrorCode::NotFound => Ok(None),
        Err(e) => Err(e),
    }
}

fn append_commit(
    repo: &Repository,
    ref_name: &str,
    message: &str,
    chain_tip: Option<&git2::Commit<'_>>,
    second_parent: Option<&git2::Commit<'_>>,
) -> Result<git2::Oid, git2::Error> {
    let sig = repo.signature()?;
    let empty_tree = repo.find_tree(git2::Oid::from_str(EMPTY_TREE_OID)?)?;

    let mut parents: Vec<&git2::Commit<'_>> = Vec::new();
    if let Some(c) = chain_tip {
        parents.push(c);
    }
    if let Some(c) = second_parent {
        parents.push(c);
    }

    repo.commit(Some(ref_name), &sig, &sig, message, &empty_tree, &parents)
}

/// Resolve a path to a blob OID by looking it up in HEAD's tree.
///
/// # Errors
///
/// Returns an error if the path does not resolve to a blob or if the repository operation fails.
pub fn blob_oid_for_path(repo: &Repository, path_str: &str) -> Result<git2::Oid, git2::Error> {
    let tree = repo.head()?.peel_to_tree()?;
    let entry = tree.get_path(std::path::Path::new(path_str))?;
    if entry.kind() != Some(git2::ObjectType::Blob) {
        return Err(git2::Error::from_str("path does not resolve to a blob"));
    }
    Ok(entry.id())
}

impl Comments for Repository {
    fn comments_on(&self, ref_name: &str) -> Result<Vec<Comment>, git2::Error> {
        let reference = match self.find_reference(ref_name) {
            Ok(r) => r,
            Err(e) if e.code() == git2::ErrorCode::NotFound => return Ok(Vec::new()),
            Err(e) => return Err(e),
        };

        let mut commit = reference.peel_to_commit()?;
        let mut comments = Vec::new();

        loop {
            comments.push(comment_from_commit(self, &commit)?);
            match commit.parent(0) {
                Ok(parent) => commit = parent,
                Err(_) => break,
            }
        }

        Ok(comments)
    }

    fn find_comment(
        &self,
        ref_name: &str,
        oid: git2::Oid,
    ) -> Result<Option<Comment>, git2::Error> {
        let reference = match self.find_reference(ref_name) {
            Ok(r) => r,
            Err(e) if e.code() == git2::ErrorCode::NotFound => return Ok(None),
            Err(e) => return Err(e),
        };

        let mut commit = reference.peel_to_commit()?;

        loop {
            if commit.id() == oid {
                return Ok(Some(comment_from_commit(self, &commit)?));
            }
            match commit.parent(0) {
                Ok(parent) => commit = parent,
                Err(_) => break,
            }
        }

        Ok(None)
    }

    fn add_comment(
        &self,
        ref_name: &str,
        anchor: &Anchor,
        body: &str,
    ) -> Result<git2::Oid, git2::Error> {
        let tip = current_tip(self, ref_name)?;
        let message = build_message(body, anchor, false, None);
        append_commit(self, ref_name, &message, tip.as_ref(), None)
    }

    fn reply_to_comment(
        &self,
        ref_name: &str,
        parent_oid: git2::Oid,
        body: &str,
    ) -> Result<git2::Oid, git2::Error> {
        let tip = current_tip(self, ref_name)?;
        let parent_commit = self.find_commit(parent_oid)?;
        let parent_comment = comment_from_commit(self, &parent_commit)?;
        let message = build_message(body, &parent_comment.anchor, false, None);
        append_commit(self, ref_name, &message, tip.as_ref(), Some(&parent_commit))
    }

    fn resolve_comment(
        &self,
        ref_name: &str,
        comment_oid: git2::Oid,
    ) -> Result<git2::Oid, git2::Error> {
        let tip = current_tip(self, ref_name)?;
        let resolved_commit = self.find_commit(comment_oid)?;
        let resolved_comment = comment_from_commit(self, &resolved_commit)?;
        let message = build_message("", &resolved_comment.anchor, true, None);
        append_commit(self, ref_name, &message, tip.as_ref(), Some(&resolved_commit))
    }

    fn edit_comment(
        &self,
        ref_name: &str,
        comment_oid: git2::Oid,
        new_body: &str,
    ) -> Result<git2::Oid, git2::Error> {
        let tip = current_tip(self, ref_name)?;
        let original_commit = self.find_commit(comment_oid)?;
        let original_comment = comment_from_commit(self, &original_commit)?;
        let message = build_message(new_body, &original_comment.anchor, false, Some(comment_oid));
        append_commit(self, ref_name, &message, tip.as_ref(), Some(&original_commit))
    }
}
