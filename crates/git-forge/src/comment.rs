//! Comment chains backed by `git-chain`.

use std::collections::HashMap;

use facet::Facet;
use git_chain::{Chain, ChainEntry};
use git2::{ObjectType, Oid, Repository};
use serde::Serialize;
use uuid::Uuid;

use crate::refs::{COMMENTS_INDEX, COMMENTS_PREFIX};
use crate::{Error, Result};

/// The anchor target for a comment.
///
/// A comment can anchor to any git object: a blob, a commit, or a tree.
/// Line range is meaningful only for blob anchors.
#[derive(Debug, Clone, Serialize, Facet)]
pub struct Anchor {
    /// OID of the anchored git object (blob, commit, or tree).
    pub oid: String,
    /// Start line — only meaningful for blob anchors.
    pub start_line: Option<u32>,
    /// End line — only meaningful for blob anchors.
    pub end_line: Option<u32>,
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
    /// OID of the comment this is a reply to.
    pub reply_to: Option<String>,
    /// Tree OID of the chain commit.
    pub tree: String,
    /// Surrounding source lines stored in the comment tree (v2).
    pub context_lines: Option<String>,
    /// UUID of the thread this comment belongs to (from `Comment-Id` trailer).
    pub thread_id: Option<String>,
}

/// Return the chain ref name for a comment thread.
#[must_use]
pub fn comment_thread_ref(thread_id: &str) -> String {
    format!("{COMMENTS_PREFIX}{thread_id}")
}

// ---------------------------------------------------------------------------
// Trailers
// ---------------------------------------------------------------------------

/// Trailer key for the `Comment-Id` (UUID v7 of the thread).
const TRAILER_COMMENT_ID: &str = "Comment-Id";

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
    TRAILER_COMMENT_ID,
];

/// Build a trailer block for thread operations.
fn format_trailers(
    anchor_oid: &str,
    anchor_range: Option<(u32, u32)>,
    thread_id: &str,
    resolved: bool,
    replaces: Option<&str>,
) -> String {
    let mut lines: Vec<String> = Vec::new();
    lines.push(format!("Anchor: {anchor_oid}"));
    if let Some((start, end)) = anchor_range {
        lines.push(format!("Anchor-Range: {start}-{end}"));
    }
    lines.push(format!("{TRAILER_COMMENT_ID}: {thread_id}"));
    if resolved {
        lines.push("Resolved: true".to_string());
    }
    if let Some(oid) = replaces {
        lines.push(format!("Replaces: {oid}"));
    }
    lines.join("\n")
}

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
    let second_parent = commit.parent_id(1).ok().map(|oid| oid.to_string());

    let (body, trailers) = parse_trailers(&entry.message);

    let anchor = trailers.get("Anchor").map(|anchor_oid| {
        let (start_line, end_line) = trailers
            .get("Anchor-Range")
            .and_then(|r| parse_anchor_range(r))
            .unwrap_or((None, None));
        Anchor {
            oid: anchor_oid.clone(),
            start_line,
            end_line,
        }
    });

    let resolved = trailers.get("Resolved").is_some_and(|v| v == "true");
    let replaces = trailers.get("Replaces").cloned();
    // Edits use the second parent to point at the replaced commit, not as
    // a reply.  Only treat the second parent as `reply_to` when there is
    // no `Replaces` trailer.
    let reply_to = if replaces.is_some() {
        None
    } else {
        second_parent
    };
    let thread_id = trailers.get(TRAILER_COMMENT_ID).cloned();

    let context_lines = read_tree_blob(repo, entry.tree, "context");

    Ok(Comment {
        oid: entry.commit.to_string(),
        body,
        author_name,
        author_email,
        timestamp,
        anchor,
        resolved,
        replaces,
        reply_to,
        tree: entry.tree.to_string(),
        context_lines,
        thread_id,
    })
}

/// Parse an anchor range string like `"42-47"` or `"42"` into `(start, end)`.
fn parse_anchor_range(range: &str) -> Option<(Option<u32>, Option<u32>)> {
    if let Some((a, b)) = range.split_once('-') {
        Some((a.parse().ok(), b.parse().ok()))
    } else {
        let n: u32 = range.parse().ok()?;
        Some((Some(n), Some(n)))
    }
}

/// Read a UTF-8 blob entry from a git tree by name.
fn read_tree_blob(repo: &Repository, tree_oid: Oid, name: &str) -> Option<String> {
    let tree = repo.find_tree(tree_oid).ok()?;
    let entry = tree.get_name(name)?;
    if entry.kind() != Some(ObjectType::Blob) {
        return None;
    }
    let blob = repo.find_blob(entry.id()).ok()?;
    String::from_utf8(blob.content().to_vec()).ok()
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

// ---------------------------------------------------------------------------
// v2 tree building
// ---------------------------------------------------------------------------

/// Build a comment tree with `body`, `anchor`, optional `context`, and
/// `anchor-content` entries.
///
/// # Errors
/// Returns an error if the anchor OID is invalid or a git operation fails.
pub fn build_comment_tree(
    repo: &Repository,
    body: &str,
    anchor: Option<&Anchor>,
    context_lines: Option<&str>,
) -> Result<Oid> {
    let mut builder = repo.treebuilder(None)?;

    // body blob
    let body_oid = repo.blob(body.as_bytes())?;
    builder.insert("body", body_oid, 0o100_644)?;

    if let Some(a) = anchor {
        // anchor TOML blob
        use std::fmt::Write as _;
        let mut toml = format!("oid = {:?}\n", a.oid);
        if let Some(s) = a.start_line {
            let _ = writeln!(toml, "start_line = {s}");
        }
        if let Some(e) = a.end_line {
            let _ = writeln!(toml, "end_line = {e}");
        }
        let anchor_blob_oid = repo.blob(toml.as_bytes())?;
        builder.insert("anchor", anchor_blob_oid, 0o100_644)?;

        // context blob (only for blob anchors with a line range)
        if let (Some(_), Some(ctx)) = (a.start_line, context_lines) {
            let ctx_oid = repo.blob(ctx.as_bytes())?;
            builder.insert("context", ctx_oid, 0o100_644)?;
        }

        // anchor-content: insert the anchored object to prevent GC
        let anchor_oid = Oid::from_str(&a.oid)?;
        let obj = repo.find_object(anchor_oid, None)?;
        let (mode, _oid) = match obj.kind() {
            Some(ObjectType::Blob) => (0o100_644, anchor_oid),
            Some(ObjectType::Tree) => (0o040_000, anchor_oid),
            Some(ObjectType::Commit) => (0o160_000, anchor_oid),
            _ => {
                return Err(Error::InvalidObjectType(
                    obj.kind()
                        .map_or_else(|| "unknown".to_string(), |k| k.to_string()),
                ));
            }
        };
        builder.insert("anchor-content", anchor_oid, mode)?;
    }

    Ok(builder.write()?)
}

// ---------------------------------------------------------------------------
// v2 OID helpers
// ---------------------------------------------------------------------------

/// Resolve an OID or short prefix to a full OID from the named ref.
fn resolve_thread_oid(repo: &Repository, ref_name: &str, prefix: &str) -> Result<Oid> {
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

/// Get the `Anchor` trailer OID from the tip commit of a thread ref.
///
/// Returns `None` if the ref doesn't exist or has no `Anchor` trailer.
fn tip_anchor_oid(repo: &Repository, ref_name: &str) -> Option<String> {
    let reference = repo.find_reference(ref_name).ok()?;
    let commit = reference.peel_to_commit().ok()?;
    let message = commit.message()?;
    let (_, trailers) = parse_trailers(message);
    trailers.get("Anchor").cloned()
}

// ---------------------------------------------------------------------------
// v2 thread operations
// ---------------------------------------------------------------------------

/// Create a new comment thread. Returns `(thread_id, root_comment)`.
///
/// # Errors
/// Returns an error if a git operation fails.
pub fn create_thread(
    repo: &Repository,
    body: &str,
    anchor: Option<&Anchor>,
    context_lines: Option<&str>,
) -> Result<(String, Comment)> {
    let thread_id = Uuid::now_v7().to_string();
    let ref_name = comment_thread_ref(&thread_id);

    let anchor_oid_str = anchor.map_or("", |a| a.oid.as_str());
    let anchor_range = anchor.and_then(|a| a.start_line.zip(a.end_line));
    let trailers = format_trailers(anchor_oid_str, anchor_range, &thread_id, false, None);
    let message = build_message(body, &trailers);
    let tree = build_comment_tree(repo, body, anchor, context_lines)?;
    let entry = repo.append(&ref_name, &message, tree, None)?;
    let comment = comment_from_chain_entry(repo, &entry)?;
    Ok((thread_id, comment))
}

/// Append a reply to an existing thread.
///
/// # Errors
/// Returns an error if the thread ref or reply-to OID cannot be found.
pub fn reply_to_thread(
    repo: &Repository,
    thread_id: &str,
    body: &str,
    reply_to_oid: &str,
    anchor: Option<&Anchor>,
    context_lines: Option<&str>,
) -> Result<Comment> {
    let ref_name = comment_thread_ref(thread_id);

    // Determine anchor for this reply: use provided anchor or inherit from root.
    let effective_anchor_oid: String;
    let effective_range: Option<(u32, u32)>;
    if let Some(a) = anchor {
        effective_anchor_oid = a.oid.clone();
        effective_range = a.start_line.zip(a.end_line);
    } else {
        effective_anchor_oid = tip_anchor_oid(repo, &ref_name).unwrap_or_default();
        effective_range = None;
    }

    let trailers = format_trailers(
        &effective_anchor_oid,
        effective_range,
        thread_id,
        false,
        None,
    );
    let message = build_message(body, &trailers);
    let tree = build_comment_tree(repo, body, anchor, context_lines)?;
    let parent = resolve_thread_oid(repo, &ref_name, reply_to_oid)?;
    let entry = repo.append(&ref_name, &message, tree, Some(parent))?;
    comment_from_chain_entry(repo, &entry)
}

/// Append a resolution to a thread.
///
/// # Errors
/// Returns an error if the thread ref or reply-to OID cannot be found.
pub fn resolve_thread(
    repo: &Repository,
    thread_id: &str,
    reply_to_oid: &str,
    message: Option<&str>,
) -> Result<Comment> {
    let ref_name = comment_thread_ref(thread_id);
    let anchor_oid = tip_anchor_oid(repo, &ref_name).unwrap_or_default();

    let body = message.unwrap_or("");
    let trailers = format_trailers(&anchor_oid, None, thread_id, true, None);
    let msg = build_message(body, &trailers);

    // Resolution tree: body only, no anchor object (inheriting anchor)
    let inherited_anchor = if anchor_oid.is_empty() {
        None
    } else {
        Some(Anchor {
            oid: anchor_oid,
            start_line: None,
            end_line: None,
        })
    };
    let tree = build_comment_tree(repo, body, inherited_anchor.as_ref(), None)?;
    let parent = resolve_thread_oid(repo, &ref_name, reply_to_oid)?;
    let entry = repo.append(&ref_name, &msg, tree, Some(parent))?;
    comment_from_chain_entry(repo, &entry)
}

/// Append an edit to a thread.
///
/// # Errors
/// Returns an error if the thread ref or original OID cannot be found.
pub fn edit_in_thread(
    repo: &Repository,
    thread_id: &str,
    original_oid: &str,
    new_body: &str,
    anchor: Option<&Anchor>,
    context_lines: Option<&str>,
) -> Result<Comment> {
    let ref_name = comment_thread_ref(thread_id);
    let parent = resolve_thread_oid(repo, &ref_name, original_oid)?;
    let parent_str = parent.to_string();

    // Use provided anchor or inherit root's anchor.
    let effective_anchor_oid: String;
    let effective_range: Option<(u32, u32)>;
    if let Some(a) = anchor {
        effective_anchor_oid = a.oid.clone();
        effective_range = a.start_line.zip(a.end_line);
    } else {
        effective_anchor_oid = tip_anchor_oid(repo, &ref_name).unwrap_or_default();
        effective_range = None;
    }

    let trailers = format_trailers(
        &effective_anchor_oid,
        effective_range,
        thread_id,
        false,
        Some(&parent_str),
    );
    let message = build_message(new_body, &trailers);
    let tree = build_comment_tree(repo, new_body, anchor, context_lines)?;
    let entry = repo.append(&ref_name, &message, tree, Some(parent))?;
    comment_from_chain_entry(repo, &entry)
}

/// List all comments in a thread (first-parent walk, tip-first).
///
/// # Errors
/// Returns an error if the ref does not exist.
pub fn list_thread_comments(repo: &Repository, thread_id: &str) -> Result<Vec<Comment>> {
    let ref_name = comment_thread_ref(thread_id);
    let entries = repo.walk(&ref_name, None)?;
    entries
        .iter()
        .map(|e| comment_from_chain_entry(repo, e))
        .collect()
}

/// Return `true` if any commit in the thread carries `Resolved: true`.
///
/// # Errors
/// Returns an error if a git operation fails.
pub fn thread_is_resolved(repo: &Repository, thread_id: &str) -> Result<bool> {
    let ref_name = comment_thread_ref(thread_id);
    let Ok(entries) = repo.walk(&ref_name, None) else {
        return Ok(false);
    };
    Ok(entries.iter().any(|e| {
        let (_, trailers) = parse_trailers(&e.message);
        trailers.get("Resolved").is_some_and(|v| v == "true")
    }))
}

/// List all thread UUIDs in the repository.
///
/// # Errors
/// Returns an error if ref enumeration fails.
pub fn list_all_thread_ids(repo: &Repository) -> Result<Vec<String>> {
    let mut ids = Vec::new();
    match repo.references_glob(&format!("{COMMENTS_PREFIX}*")) {
        Ok(refs) => {
            for r in refs.flatten() {
                if let Some(name) = r.name()
                    && let Some(id) = name.strip_prefix(COMMENTS_PREFIX)
                    && !id.contains('/')
                {
                    ids.push(id.to_string());
                }
            }
        }
        Err(e) if e.code() == git2::ErrorCode::NotFound => {}
        Err(e) => return Err(e.into()),
    }
    Ok(ids)
}

/// Find threads containing comments anchored to a given object OID.
///
/// Uses the signed index if present, falls back to scanning tip-commit trailers.
///
/// # Errors
/// Returns an error if a git operation fails.
pub fn find_threads_by_object(repo: &Repository, oid: &str) -> Result<Vec<String>> {
    // Try the index first.
    if let Some(ids) = index_lookup(repo, oid)? {
        return Ok(ids);
    }

    // Fallback: scan tip commits of all thread refs.
    let thread_ids = list_all_thread_ids(repo)?;
    let mut result = Vec::new();
    for tid in &thread_ids {
        let ref_name = comment_thread_ref(tid);
        if let Some(anchor) = tip_anchor_oid(repo, &ref_name)
            && anchor == oid
        {
            result.push(tid.clone());
            continue;
        }
        // Full walk for threads where replies may anchor to a different object.
        if let Ok(entries) = repo.walk(&ref_name, None) {
            for entry in &entries {
                let (_, trailers) = parse_trailers(&entry.message);
                if trailers.get("Anchor").is_some_and(|a| a == oid) {
                    result.push(tid.clone());
                    break;
                }
            }
        }
    }
    Ok(result)
}

// ---------------------------------------------------------------------------
// Index
// ---------------------------------------------------------------------------

/// Rebuild the `refs/forge/index/comments-by-object` index from scratch.
///
/// # Errors
/// Returns an error if a git operation fails.
pub fn rebuild_comments_index(repo: &Repository) -> Result<()> {
    use std::collections::{BTreeMap, BTreeSet};

    // Collect object_oid → set of thread_ids mappings.
    let mut index: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();

    let thread_ids = list_all_thread_ids(repo)?;
    for tid in &thread_ids {
        let ref_name = comment_thread_ref(tid);
        if let Ok(entries) = repo.walk(&ref_name, None) {
            for entry in &entries {
                let (_, trailers) = parse_trailers(&entry.message);
                if let Some(anchor_oid) = trailers.get("Anchor") {
                    index
                        .entry(anchor_oid.clone())
                        .or_default()
                        .insert(tid.clone());
                }
            }
        }
    }

    // Build fanout tree: first 2 chars → rest of oid → blob of thread UUIDs.
    let mut root_builder = repo.treebuilder(None)?;

    // Group by 2-char prefix.
    let mut by_prefix: BTreeMap<String, BTreeMap<String, BTreeSet<String>>> = BTreeMap::new();
    for (oid, tids) in &index {
        if oid.len() < 3 {
            continue;
        }
        let (prefix, rest) = oid.split_at(2);
        by_prefix
            .entry(prefix.to_string())
            .or_default()
            .insert(rest.to_string(), tids.clone());
    }

    for (prefix, entries) in &by_prefix {
        let mut dir_builder = repo.treebuilder(None)?;
        for (rest, tids) in entries {
            let blob_content: String = tids.iter().fold(String::new(), |mut acc, t| {
                acc.push_str(t);
                acc.push('\n');
                acc
            });
            let blob_oid = repo.blob(blob_content.as_bytes())?;
            dir_builder.insert(rest, blob_oid, 0o100_644)?;
        }
        let dir_oid = dir_builder.write()?;
        root_builder.insert(prefix, dir_oid, 0o040_000)?;
    }

    let root_tree_oid = root_builder.write()?;
    let root_tree = repo.find_tree(root_tree_oid)?;
    let sig = repo.signature()?;

    // Determine parent commit for the index ref.
    let parent = repo
        .find_reference(COMMENTS_INDEX)
        .ok()
        .and_then(|r| r.peel_to_commit().ok());
    let parents: Vec<&git2::Commit<'_>> = parent.iter().collect();

    // Sign if a signing key is configured.
    let cfg = repo.config()?;
    let signing_key = cfg.get_string("user.signingkey").ok();
    if let Some(_key) = signing_key {
        // GPG signing via git2 is not straightforward; commit unsigned for now
        // and let the server or post-receive hook sign if needed.
        repo.commit(
            Some(COMMENTS_INDEX),
            &sig,
            &sig,
            "forge: rebuild comments-by-object index",
            &root_tree,
            &parents,
        )?;
    } else {
        repo.commit(
            Some(COMMENTS_INDEX),
            &sig,
            &sig,
            "forge: rebuild comments-by-object index",
            &root_tree,
            &parents,
        )?;
    }

    Ok(())
}

/// Look up thread UUIDs by object OID using the index.
///
/// Returns `None` if the index ref doesn't exist.
///
/// # Errors
/// Returns an error if a git operation fails.
pub fn index_lookup(repo: &Repository, oid: &str) -> Result<Option<Vec<String>>> {
    use git2::ErrorCode;

    if oid.len() < 3 {
        return Ok(None);
    }

    let reference = match repo.find_reference(COMMENTS_INDEX) {
        Ok(r) => r,
        Err(e) if e.code() == ErrorCode::NotFound => return Ok(None),
        Err(e) => return Err(e.into()),
    };

    let tree = reference.peel_to_commit()?.tree()?;
    let (prefix, rest) = oid.split_at(2);

    let Some(dir_entry) = tree.get_name(prefix) else {
        return Ok(Some(Vec::new()));
    };
    if dir_entry.kind() != Some(ObjectType::Tree) {
        return Ok(Some(Vec::new()));
    }
    let dir_tree = repo.find_tree(dir_entry.id())?;

    let Some(leaf_entry) = dir_tree.get_name(rest) else {
        return Ok(Some(Vec::new()));
    };
    if leaf_entry.kind() != Some(ObjectType::Blob) {
        return Ok(Some(Vec::new()));
    }
    let blob = repo.find_blob(leaf_entry.id())?;
    let content = String::from_utf8_lossy(blob.content());
    let ids: Vec<String> = content
        .lines()
        .filter(|l| !l.is_empty())
        .map(str::to_string)
        .collect();
    Ok(Some(ids))
}
