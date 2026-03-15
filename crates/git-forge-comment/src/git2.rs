//! `git2::Repository` implementation of [`Comments`].

use git2::Repository;

use crate::{Anchor, Comment, Comments};

const EMPTY_TREE_OID: &str = "4b825dc642cb6eb9a060e54bf8d69288fbee4904";

struct Trailers {
    anchor_oid: Option<git2::Oid>,
    anchor_range: Option<(u32, u32)>,
    anchor_end: Option<git2::Oid>,
    resolved: bool,
}

fn parse_trailers(msg: &str) -> Trailers {
    let mut t = Trailers { anchor_oid: None, anchor_range: None, anchor_end: None, resolved: false };
    for line in msg.lines() {
        if let Some(v) = line.strip_prefix("Anchor: ") {
            t.anchor_oid = git2::Oid::from_str(v.trim()).ok();
        } else if let Some(v) = line.strip_prefix("Anchor-Range: ") {
            if let Some((s, e)) = v.trim().split_once('-') {
                if let (Ok(start), Ok(end)) = (s.parse::<u32>(), e.parse::<u32>()) {
                    t.anchor_range = Some((start, end));
                }
            }
        } else if let Some(v) = line.strip_prefix("Anchor-End: ") {
            t.anchor_end = git2::Oid::from_str(v.trim()).ok();
        } else if let Some(v) = line.strip_prefix("Resolved: ") {
            t.resolved = v.trim() == "true";
        }
    }
    t
}

fn body_from_message(msg: &str) -> String {
    let is_trailer = |line: &str| -> bool {
        line.find(": ")
            .map(|pos| {
                let key = &line[..pos];
                !key.is_empty() && key.chars().all(|c| c.is_alphanumeric() || c == '-')
            })
            .unwrap_or(false)
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
        Some(git2::ObjectType::Blob) => Ok(Anchor::Blob { oid, line_range: t.anchor_range }),
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

    Ok(Comment { oid: commit.id(), anchor, body, resolved: trailers.resolved, parent_oid })
}

fn build_message(body: &str, anchor: &Anchor, resolved: bool) -> String {
    let mut msg = body.trim_end().to_string();
    msg.push_str("\n\n");

    match anchor {
        Anchor::Blob { oid, line_range } => {
            msg.push_str(&format!("Anchor: {oid}\n"));
            if let Some((start, end)) = line_range {
                msg.push_str(&format!("Anchor-Range: {start}-{end}\n"));
            }
        }
        Anchor::Commit(oid) => msg.push_str(&format!("Anchor: {oid}\n")),
        Anchor::Tree(oid) => msg.push_str(&format!("Anchor: {oid}\n")),
        Anchor::CommitRange { start, end } => {
            msg.push_str(&format!("Anchor: {start}\n"));
            msg.push_str(&format!("Anchor-End: {end}\n"));
        }
    }

    if resolved {
        msg.push_str("Resolved: true\n");
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
        let message = build_message(body, anchor, false);
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
        let message = build_message(body, &parent_comment.anchor, false);
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
        let message = build_message("", &resolved_comment.anchor, true);
        append_commit(self, ref_name, &message, tip.as_ref(), Some(&resolved_commit))
    }
}
