//! `git2::Repository` implementation of [`Issues`].

use git2::Repository;
use git_forge_comment::{Anchor, Comments, issue_comments_ref};

use crate::{ISSUES_REF_PREFIX, Issue, IssueMeta, IssueOpts, IssueState, Issues};

// ---------------------------------------------------------------------------
// Reading helpers
// ---------------------------------------------------------------------------

fn blob_content<'repo>(
    repo: &'repo Repository,
    tree: &git2::Tree<'repo>,
    name: &str,
) -> Result<Option<String>, git2::Error> {
    match tree.get_name(name) {
        None => Ok(None),
        Some(entry) => {
            let obj = entry.to_object(repo)?;
            let blob = obj
                .as_blob()
                .ok_or_else(|| git2::Error::from_str(&format!("{name} is not a blob")))?;
            Ok(Some(
                std::str::from_utf8(blob.content())
                    .unwrap_or("")
                    .trim_end()
                    .to_string(),
            ))
        }
    }
}

fn read_state(repo: &Repository, tree: &git2::Tree<'_>) -> Result<IssueState, git2::Error> {
    match blob_content(repo, tree, "state")?.as_deref() {
        Some("closed") => Ok(IssueState::Closed),
        _ => Ok(IssueState::Open),
    }
}

fn read_labels(repo: &Repository, tree: &git2::Tree<'_>) -> Result<Vec<String>, git2::Error> {
    let Some(entry) = tree.get_name("labels") else {
        return Ok(Vec::new());
    };
    let obj = entry.to_object(repo)?;
    let subtree = obj
        .as_tree()
        .ok_or_else(|| git2::Error::from_str("labels is not a tree"))?;
    let mut labels: Vec<String> = subtree
        .iter()
        .filter_map(|e| e.name().map(str::to_string))
        .collect();
    labels.sort();
    Ok(labels)
}

fn read_comments(repo: &Repository, id: u64) -> Result<Vec<(String, String)>, git2::Error> {
    let comments = repo.comments_on(&issue_comments_ref(id))?;
    Ok(comments.into_iter().map(|c| (c.oid.to_string(), c.body)).collect())
}

fn issue_from_ref(
    repo: &Repository,
    reference: &git2::Reference<'_>,
    ref_prefix: &str,
) -> Result<Option<Issue>, git2::Error> {
    let Some(ref_name) = reference.name() else {
        return Ok(None);
    };

    let Some(id_str) = ref_name.strip_prefix(ref_prefix) else {
        return Ok(None);
    };

    // Skip nested refs (e.g. sub-directories).
    if id_str.contains('/') {
        return Ok(None);
    }

    let id: u64 = match id_str.parse() {
        Ok(n) => n,
        Err(_) => return Ok(None),
    };

    let commit = reference.peel_to_commit()?;
    let tree = commit.tree()?;

    let author = blob_content(repo, &tree, "author")?.unwrap_or_default();
    let title = blob_content(repo, &tree, "title")?.unwrap_or_default();
    let state = read_state(repo, &tree)?;
    let labels = read_labels(repo, &tree)?;

    let body = blob_content(repo, &tree, "body")?.unwrap_or_default();
    let comments = read_comments(repo, id)?;

    Ok(Some(Issue {
        id,
        meta: IssueMeta {
            author,
            title,
            state,
            labels,
        },
        body,
        comments,
    }))
}

// ---------------------------------------------------------------------------
// Trait impl
// ---------------------------------------------------------------------------

impl Issues for Repository {
    fn list_issues(&self, opts: Option<&IssueOpts>) -> Result<Vec<Issue>, git2::Error> {
        let prefix = opts.map_or(ISSUES_REF_PREFIX, |o| o.ref_prefix.as_str());

        let mut issues = Vec::new();
        for reference in self.references_glob(&format!("{prefix}*"))? {
            let reference = reference?;
            if let Some(issue) = issue_from_ref(self, &reference, prefix)? {
                issues.push(issue);
            }
        }
        issues.sort_by_key(|i| i.id);
        Ok(issues)
    }

    fn list_issues_by_state(
        &self,
        state: IssueState,
        opts: Option<&IssueOpts>,
    ) -> Result<Vec<Issue>, git2::Error> {
        let prefix = opts.map_or(ISSUES_REF_PREFIX, |o| o.ref_prefix.as_str());

        let mut issues = Vec::new();
        for reference in self.references_glob(&format!("{prefix}*"))? {
            let reference = reference?;
            let ref_name = reference
                .name()
                .ok_or_else(|| git2::Error::from_str("ref name is not valid UTF-8"))?;
            let id_str = &ref_name[prefix.len()..];
            if id_str.parse::<u64>().is_err() {
                return Err(git2::Error::from_str(&format!(
                    "ref {ref_name} has non-numeric id; delete it to continue"
                )));
            }
            let commit = reference.peel_to_commit()?;
            let tree = commit.tree()?;
            // TODO perhaps we just continue if no state found, instead of Err?
            if read_state(self, &tree)? != state {
                continue;
            }
            if let Some(issue) = issue_from_ref(self, &reference, prefix)? {
                issues.push(issue);
            }
        }
        issues.sort_by_key(|i| i.id);
        Ok(issues)
    }

    fn find_issue(&self, id: u64, opts: Option<&IssueOpts>) -> Result<Option<Issue>, git2::Error> {
        let prefix = opts.map_or(ISSUES_REF_PREFIX, |o| o.ref_prefix.as_str());
        let ref_name = format!("{prefix}{id}");
        match self.find_reference(&ref_name) {
            Ok(reference) => issue_from_ref(self, &reference, prefix),
            Err(e) if e.code() == git2::ErrorCode::NotFound => Ok(None),
            Err(e) => Err(e),
        }
    }

    fn create_issue(
        &self,
        title: &str,
        body: &str,
        labels: &[String],
        _assignees: &[String],
        opts: Option<&IssueOpts>,
    ) -> Result<u64, git2::Error> {
        let prefix = opts.map_or(ISSUES_REF_PREFIX, |o| o.ref_prefix.as_str());

        // Determine next ID: max existing + 1, or 1.
        let next_id = {
            let mut max = 0u64;
            for reference in self.references_glob(&format!("{prefix}*"))? {
                let reference = reference?;
                if let Some(name) = reference.name()
                    && let Some(id_str) = name.strip_prefix(prefix)
                    && !id_str.contains('/')
                    && let Ok(n) = id_str.parse::<u64>()
                    && n > max
                {
                    max = n;
                }
            }
            max + 1
        };

        let sig = self.signature()?;
        let empty_blob = self.blob(b"")?;

        // labels/ subtree
        let labels_tree = {
            let mut tb = self.treebuilder(None)?;
            for label in labels {
                tb.insert(label, empty_blob, 0o100_644)?;
            }
            tb.write()?
        };

        let author_blob = self.blob(sig.name().unwrap_or("").as_bytes())?;
        let title_blob = self.blob(title.as_bytes())?;
        let state_blob = self.blob(b"open")?;
        let body_blob = self.blob(body.as_bytes())?;

        let tree_oid = {
            let mut tb = self.treebuilder(None)?;
            tb.insert("author", author_blob, 0o100_644)?;
            tb.insert("title", title_blob, 0o100_644)?;
            tb.insert("state", state_blob, 0o100_644)?;
            tb.insert("body", body_blob, 0o100_644)?;
            tb.insert("labels", labels_tree, 0o040_000)?;
            tb.write()?
        };

        let tree = self.find_tree(tree_oid)?;
        let ref_name = format!("{prefix}{next_id}");
        let message = format!("create issue {next_id}");
        self.commit(Some(&ref_name), &sig, &sig, &message, &tree, &[])?;

        Ok(next_id)
    }

    fn update_issue(
        &self,
        id: u64,
        title: Option<&str>,
        body: Option<&str>,
        labels: Option<&[String]>,
        _assignees: Option<&[String]>,
        state: Option<IssueState>,
        opts: Option<&IssueOpts>,
    ) -> Result<(), git2::Error> {
        let prefix = opts.map_or(ISSUES_REF_PREFIX, |o| o.ref_prefix.as_str());

        // Find the existing issue
        let existing = self.find_issue(id, opts)?;
        let mut issue =
            existing.ok_or_else(|| git2::Error::from_str(&format!("issue {id} not found")))?;

        // Apply updates
        if let Some(new_title) = title {
            issue.meta.title = new_title.to_string();
        }
        if let Some(new_body) = body {
            issue.body = new_body.to_string();
        }
        if let Some(new_labels) = labels {
            issue.meta.labels = new_labels.to_vec();
        }
        if let Some(new_state) = state {
            issue.meta.state = new_state;
        }

        // Build the new tree
        let sig = self.signature()?;
        let empty_blob = self.blob(b"")?;

        // labels/ subtree
        let labels_tree = {
            let mut tb = self.treebuilder(None)?;
            for label in &issue.meta.labels {
                tb.insert(label, empty_blob, 0o100_644)?;
            }
            tb.write()?
        };

        let author_blob = self.blob(issue.meta.author.as_bytes())?;
        let title_blob = self.blob(issue.meta.title.as_bytes())?;
        let state_blob = self.blob(issue.meta.state.as_str().as_bytes())?;
        let body_blob = self.blob(issue.body.as_bytes())?;

        let tree_oid = {
            let mut tb = self.treebuilder(None)?;
            tb.insert("author", author_blob, 0o100_644)?;
            tb.insert("title", title_blob, 0o100_644)?;
            tb.insert("state", state_blob, 0o100_644)?;
            tb.insert("body", body_blob, 0o100_644)?;
            tb.insert("labels", labels_tree, 0o040_000)?;
            tb.write()?
        };

        let tree = self.find_tree(tree_oid)?;
        let ref_name = format!("{prefix}{id}");

        // Get the current commit for the parent
        let reference = self.find_reference(&ref_name)?;
        let parent_commit = reference.peel_to_commit()?;

        let message = format!("update issue {id}");
        self.commit(
            Some(&ref_name),
            &sig,
            &sig,
            &message,
            &tree,
            &[&parent_commit],
        )?;

        Ok(())
    }

    fn add_issue_comment(
        &self,
        id: u64,
        _author: &str,
        body: &str,
        opts: Option<&IssueOpts>,
    ) -> Result<(), git2::Error> {
        let prefix = opts.map_or(ISSUES_REF_PREFIX, |o| o.ref_prefix.as_str());
        let issue_ref_name = format!("{prefix}{id}");
        let commit_oid = self.find_reference(&issue_ref_name)?.peel_to_commit()?.id();
        let anchor = Anchor::Commit(commit_oid);
        self.add_comment(&issue_comments_ref(id), &anchor, body)?;
        Ok(())
    }
}
