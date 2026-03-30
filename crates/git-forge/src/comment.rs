//! Comment chains backed by `git-chain`.

use std::collections::HashMap;

use facet::Facet;
use git_chain::{Chain, ChainEntry};
use git2::{Oid, Repository};
use serde::Serialize;

use crate::refs::{ISSUE_COMMENTS_PREFIX, OBJECT_COMMENTS_PREFIX, REVIEW_COMMENTS_PREFIX};
use crate::{Error, Result};

/// The anchor target for a comment.
#[derive(Debug, Clone, Serialize, Facet)]
#[repr(u8)]
pub enum Anchor {
    /// A single object (blob or commit), with an optional line range.
    Object {
        /// The target object OID.
        oid: String,
        /// Optional file path within the target (for commit-anchored comments).
        path: Option<String>,
        /// Optional line range, e.g. `"42-47"`.
        range: Option<String>,
    },
    /// A commit range.
    CommitRange {
        /// Start commit OID.
        start: String,
        /// End commit OID.
        end: String,
    },
}

/// A single comment event in a chain.
#[derive(Debug, Clone, Serialize, Facet)]
pub struct Comment {
    /// OID of the chain commit for this comment.
    pub oid: String,
    /// Comment text.
    pub body: String,
    /// Author name.
    pub author_name: String,
    /// Author email.
    pub author_email: String,
    /// Unix timestamp (seconds since epoch).
    pub timestamp: i64,
    /// Optional anchor pointing to a target object.
    pub anchor: Option<Anchor>,
    /// Whether this comment resolves a thread.
    pub resolved: bool,
    /// OID of the original comment this replaces (edit marker).
    pub replaces: Option<String>,
    /// OID of the comment this was migrated from (cross-chain carry-forward).
    pub migrated_from: Option<String>,
    /// OID of the comment this is a reply to.
    pub reply_to: Option<String>,
    /// Tree OID of the chain commit.
    pub tree: String,
}

/// Return the chain ref name for issue comments.
#[must_use]
pub fn issue_comment_ref(oid: &str) -> String {
    format!("{ISSUE_COMMENTS_PREFIX}{oid}")
}

/// Return the chain ref name for review comments.
#[must_use]
pub fn review_comment_ref(oid: &str) -> String {
    format!("{REVIEW_COMMENTS_PREFIX}{oid}")
}

/// Return the chain ref name for standalone object comments.
#[must_use]
pub fn object_comment_ref(oid: &str) -> String {
    format!("{OBJECT_COMMENTS_PREFIX}{oid}")
}

/// Build a trailer block from anchor, resolved flag, replaces OID, and
/// migrated-from OID.
#[must_use]
pub fn format_trailers(
    anchor: Option<&Anchor>,
    resolved: bool,
    replaces: Option<&str>,
    migrated_from: Option<&str>,
) -> String {
    let mut lines: Vec<String> = Vec::new();
    if let Some(a) = anchor {
        match a {
            Anchor::Object { oid, path, range } => {
                lines.push(format!("Anchor: {oid}"));
                if let Some(p) = path {
                    lines.push(format!("Anchor-Path: {p}"));
                }
                if let Some(r) = range {
                    lines.push(format!("Anchor-Range: {r}"));
                }
            }
            Anchor::CommitRange { start, end } => {
                lines.push(format!("Anchor: {start}"));
                lines.push(format!("Anchor-End: {end}"));
            }
        }
    }
    if resolved {
        lines.push("Resolved: true".to_string());
    }
    if let Some(oid) = replaces {
        lines.push(format!("Replaces: {oid}"));
    }
    if let Some(oid) = migrated_from {
        lines.push(format!("Migrated-From: {oid}"));
    }
    lines.join("\n")
}

/// Known trailer keys recognized by the comment system.
const KNOWN_TRAILER_KEYS: &[&str] = &[
    "Anchor",
    "Anchor-Path",
    "Anchor-Range",
    "Anchor-End",
    "Resolved",
    "Replaces",
    "Migrated-From",
    "Github-Id",
];

/// Split a commit message into `(body, trailers)`.
///
/// The trailer block is the last paragraph where every non-empty line
/// is a known forge trailer (`Key: value`). Unknown keys are left in
/// the body so that user-authored text is never misinterpreted.
#[must_use]
pub fn parse_trailers(message: &str) -> (String, HashMap<String, String>) {
    let paragraphs: Vec<&str> = message.split("\n\n").collect();

    let is_trailer_para = |para: &str| -> bool {
        let has_content = para.lines().any(|l| !l.is_empty());
        has_content
            && para.lines().all(|line| {
                line.is_empty() || {
                    let Some((key, _val)) = line.split_once(": ") else {
                        return false;
                    };
                    KNOWN_TRAILER_KEYS.contains(&key)
                }
            })
    };

    if let Some(last) = paragraphs.last()
        && is_trailer_para(last)
    {
        let mut trailers = HashMap::new();
        for line in last.lines() {
            if let Some((k, v)) = line.split_once(": ") {
                trailers.insert(k.to_string(), v.to_string());
            }
        }
        let body = if paragraphs.len() > 1 {
            paragraphs[..paragraphs.len() - 1].join("\n\n")
        } else {
            String::new()
        };
        return (body.trim().to_string(), trailers);
    }

    (message.trim().to_string(), HashMap::new())
}

/// Construct a `Comment` from a `ChainEntry`.
///
/// # Errors
/// Returns an error if the commit cannot be found in `repo`.
pub fn comment_from_chain_entry(repo: &Repository, entry: &ChainEntry) -> Result<Comment> {
    let commit = repo.find_commit(entry.commit)?;
    let author = commit.author();
    let author_name = author.name().unwrap_or("").to_string();
    let author_email = author.email().unwrap_or("").to_string();
    let timestamp = author.when().seconds();
    let reply_to = commit.parent_id(1).ok().map(|oid| oid.to_string());

    let (body, trailers) = parse_trailers(&entry.message);

    let anchor = if let Some(anchor_oid) = trailers.get("Anchor") {
        if let Some(end) = trailers.get("Anchor-End") {
            Some(Anchor::CommitRange {
                start: anchor_oid.clone(),
                end: end.clone(),
            })
        } else {
            Some(Anchor::Object {
                oid: anchor_oid.clone(),
                path: trailers.get("Anchor-Path").cloned(),
                range: trailers.get("Anchor-Range").cloned(),
            })
        }
    } else {
        None
    };

    let resolved = trailers.get("Resolved").is_some_and(|v| v == "true");
    let replaces = trailers.get("Replaces").cloned();
    let migrated_from = trailers.get("Migrated-From").cloned();

    Ok(Comment {
        oid: entry.commit.to_string(),
        body,
        author_name,
        author_email,
        timestamp,
        anchor,
        resolved,
        replaces,
        migrated_from,
        reply_to,
        tree: entry.tree.to_string(),
    })
}

fn build_message(body: &str, trailers: &str) -> String {
    if trailers.is_empty() {
        body.to_string()
    } else if body.is_empty() {
        trailers.to_string()
    } else {
        format!("{body}\n\n{trailers}")
    }
}

/// Append a new top-level comment to a chain.
///
/// # Errors
/// Returns an error if the git operation fails.
pub fn add_comment(
    repo: &Repository,
    ref_name: &str,
    body: &str,
    anchor: Option<&Anchor>,
) -> Result<Comment> {
    let trailers = format_trailers(anchor, false, None, None);
    let message = build_message(body, &trailers);
    let tree = repo.build_tree(&[])?;
    let entry = repo.append(ref_name, &message, tree, None)?;
    comment_from_chain_entry(repo, &entry)
}

/// Resolve an OID prefix to a full OID from the comment chain.
fn resolve_comment_oid(repo: &Repository, ref_name: &str, prefix: &str) -> Result<Oid> {
    if prefix.len() == 40 {
        return Ok(Oid::from_str(prefix)?);
    }
    let entries = repo.walk(ref_name, None)?;
    let matches: Vec<_> = entries
        .iter()
        .filter(|e| e.commit.to_string().starts_with(prefix))
        .collect();
    match matches.len() {
        0 => Err(Error::NotFound(prefix.to_string())),
        1 => Ok(matches[0].commit),
        _ => Err(Error::Ambiguous(prefix.to_string())),
    }
}

/// Append a reply to an existing comment.
///
/// # Errors
/// Returns an error if the git operation fails.
pub fn add_reply(
    repo: &Repository,
    ref_name: &str,
    body: &str,
    reply_to_oid: &str,
    anchor: Option<&Anchor>,
) -> Result<Comment> {
    let trailers = format_trailers(anchor, false, None, None);
    let message = build_message(body, &trailers);
    let tree = repo.build_tree(&[])?;
    let parent = resolve_comment_oid(repo, ref_name, reply_to_oid)?;
    let entry = repo.append(ref_name, &message, tree, Some(parent))?;
    comment_from_chain_entry(repo, &entry)
}

/// Append a resolution marker to a thread.
///
/// # Errors
/// Returns an error if the git operation fails.
pub fn resolve_comment(
    repo: &Repository,
    ref_name: &str,
    reply_to_oid: &str,
    message: Option<&str>,
) -> Result<Comment> {
    let body = message.unwrap_or("");
    let trailers = format_trailers(None, true, None, None);
    let msg = build_message(body, &trailers);
    let tree = repo.build_tree(&[])?;
    let parent = resolve_comment_oid(repo, ref_name, reply_to_oid)?;
    let entry = repo.append(ref_name, &msg, tree, Some(parent))?;
    comment_from_chain_entry(repo, &entry)
}

/// Append an edit marker that supersedes an original comment.
///
/// # Errors
/// Returns an error if the git operation fails.
pub fn edit_comment(
    repo: &Repository,
    ref_name: &str,
    original_oid: &str,
    new_body: &str,
    anchor: Option<&Anchor>,
) -> Result<Comment> {
    let parent = resolve_comment_oid(repo, ref_name, original_oid)?;
    let parent_str = parent.to_string();
    let trailers = format_trailers(anchor, false, Some(&parent_str), None);
    let message = build_message(new_body, &trailers);
    let tree = repo.build_tree(&[])?;
    let entry = repo.append(ref_name, &message, tree, Some(parent))?;
    comment_from_chain_entry(repo, &entry)
}

/// List all comments in a chain (tip-first order).
///
/// # Errors
/// Returns an error if the git operation fails.
pub fn list_comments(repo: &Repository, ref_name: &str) -> Result<Vec<Comment>> {
    let entries = repo.walk(ref_name, None)?;
    entries
        .iter()
        .map(|e| comment_from_chain_entry(repo, e))
        .collect()
}

/// List all comments in a thread rooted at `root_oid`.
///
/// # Errors
/// Returns an error if the git operation fails.
pub fn list_thread(repo: &Repository, ref_name: &str, root_oid: &str) -> Result<Vec<Comment>> {
    let oid = resolve_comment_oid(repo, ref_name, root_oid)?;
    let entries = repo.walk(ref_name, Some(oid))?;
    entries
        .iter()
        .map(|e| comment_from_chain_entry(repo, e))
        .collect()
}

/// Migrate a comment to a different chain, recording the original OID via
/// the `Migrated-From` trailer.
///
/// # Errors
/// Returns an error if the git operation fails.
pub fn migrate_comment(
    repo: &Repository,
    target_ref: &str,
    body: &str,
    anchor: Option<&Anchor>,
    migrated_from: &str,
) -> Result<Comment> {
    let trailers = format_trailers(anchor, false, None, Some(migrated_from));
    let message = build_message(body, &trailers);
    let tree = repo.build_tree(&[])?;
    let entry = repo.append(target_ref, &message, tree, None)?;
    comment_from_chain_entry(repo, &entry)
}
