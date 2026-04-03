//! Issue entity CRUD backed by `git-ledger`.

use facet::Facet;
use git_ledger::{IdStrategy, Ledger, LedgerEntry, Mutation};
use serde::Serialize;

use crate::index::{display_id_for_oid, index_upsert, read_index, resolve_oid};
use crate::refs::{ISSUE_INDEX, ISSUE_PREFIX};
use crate::{Error, Result, Store};

/// The open/closed lifecycle state of an issue.
#[derive(Debug, Clone, Serialize, Facet, PartialEq, Eq)]
#[repr(u8)]
#[serde(rename_all = "lowercase")]
pub enum IssueState {
    /// Issue is open and active.
    Open,
    /// Issue has been closed.
    Closed,
}

impl IssueState {
    /// Return the canonical string representation.
    #[must_use]
    pub fn as_str(&self) -> &'static str {
        match self {
            IssueState::Open => "open",
            IssueState::Closed => "closed",
        }
    }
}

impl std::str::FromStr for IssueState {
    type Err = Error;

    fn from_str(s: &str) -> Result<Self> {
        match s {
            "open" => Ok(IssueState::Open),
            "closed" => Ok(IssueState::Closed),
            _ => Err(Error::InvalidState(s.to_string())),
        }
    }
}

/// A forge issue backed by a git-ledger entity ref.
#[derive(Debug, Clone, Serialize, Facet)]
pub struct Issue {
    /// Permanent identity: the OID of the initial commit on the entity ref.
    pub oid: String,
    /// Display ID (`"3"` for local, `"GH1"` for GitHub-imported). `None` while pending sync.
    pub display_id: Option<String>,
    /// Issue title.
    pub title: String,
    /// Current state.
    pub state: IssueState,
    /// Body in Markdown.
    pub body: String,
    /// Label names attached to this issue.
    pub labels: Vec<String>,
    /// Contributor IDs assigned to this issue.
    pub assignees: Vec<String>,
}

fn issue_from_entry(entry: &LedgerEntry, display_id: Option<String>) -> Result<Issue> {
    let mut title = String::new();
    let mut state = IssueState::Open;
    let mut body = String::new();
    let mut labels = Vec::new();
    let mut assignees = Vec::new();

    for (name, value) in &entry.fields {
        let text = || String::from_utf8_lossy(value).into_owned();
        match name.as_str() {
            "title" => title = text(),
            "state" => state = text().parse()?,
            "body" => body = text(),
            n if n.starts_with("labels/") => {
                labels.push(n["labels/".len()..].to_string());
            }
            n if n.starts_with("assignees/") => {
                assignees.push(n["assignees/".len()..].to_string());
            }
            _ => {}
        }
    }

    Ok(Issue {
        oid: entry.id.clone(),
        display_id,
        title,
        state,
        body,
        labels,
        assignees,
    })
}

impl Store<'_> {
    /// Create a new issue, writing an OID-keyed entity ref and staging it in the index.
    ///
    /// # Errors
    /// Returns an error if a git operation fails.
    pub fn create_issue(
        &self,
        title: &str,
        body: &str,
        labels: &[&str],
        assignees: &[&str],
    ) -> Result<Issue> {
        let label_paths: Vec<String> = labels.iter().map(|l| format!("labels/{l}")).collect();
        let assignee_paths: Vec<String> =
            assignees.iter().map(|a| format!("assignees/{a}")).collect();

        let mut mutations: Vec<Mutation<'_>> = vec![
            Mutation::Set("title", title.as_bytes()),
            Mutation::Set("state", b"open"),
            Mutation::Set("body", body.as_bytes()),
        ];
        for p in &label_paths {
            mutations.push(Mutation::Set(p.as_str(), b""));
        }
        for p in &assignee_paths {
            mutations.push(Mutation::Set(p.as_str(), b""));
        }

        let entry = self.repo.create(
            ISSUE_PREFIX,
            &IdStrategy::CommitOid,
            &mutations,
            "create issue",
            None,
        )?;

        Ok(Issue {
            oid: entry.id,
            display_id: None,
            title: title.to_string(),
            state: IssueState::Open,
            body: body.to_string(),
            labels: labels
                .iter()
                .map(std::string::ToString::to_string)
                .collect(),
            assignees: assignees
                .iter()
                .map(std::string::ToString::to_string)
                .collect(),
        })
    }

    /// Create an issue with a custom git author, used when importing from an external source.
    ///
    /// `display_id` is written to the index immediately (no "pending" stage).
    ///
    /// # Errors
    /// Returns an error if a git operation fails.
    #[allow(clippy::too_many_arguments)]
    pub fn create_issue_imported(
        &self,
        title: &str,
        body: &str,
        labels: &[&str],
        assignees: &[&str],
        display_id: &str,
        author: &git2::Signature<'_>,
        source: &str,
    ) -> Result<Issue> {
        let label_paths: Vec<String> = labels.iter().map(|l| format!("labels/{l}")).collect();
        let assignee_paths: Vec<String> =
            assignees.iter().map(|a| format!("assignees/{a}")).collect();

        let mut mutations: Vec<Mutation<'_>> = vec![
            Mutation::Set("title", title.as_bytes()),
            Mutation::Set("state", b"open"),
            Mutation::Set("body", body.as_bytes()),
            Mutation::Set("source/url", source.as_bytes()),
        ];
        for p in &label_paths {
            mutations.push(Mutation::Set(p.as_str(), b""));
        }
        for p in &assignee_paths {
            mutations.push(Mutation::Set(p.as_str(), b""));
        }

        let entry = self.repo.create(
            ISSUE_PREFIX,
            &IdStrategy::CommitOid,
            &mutations,
            "forge: create issue",
            Some(author),
        )?;

        index_upsert(self.repo, ISSUE_INDEX, &[(display_id, &entry.id)])?;

        Ok(Issue {
            oid: entry.id.clone(),
            display_id: Some(display_id.to_string()),
            title: title.to_string(),
            state: IssueState::Open,
            body: body.to_string(),
            labels: labels
                .iter()
                .map(std::string::ToString::to_string)
                .collect(),
            assignees: assignees
                .iter()
                .map(std::string::ToString::to_string)
                .collect(),
        })
    }

    /// Fetch a single issue by display ID or OID prefix.
    ///
    /// # Errors
    /// Returns [`Error::NotFound`] if the issue does not exist, or a git error on failure.
    pub fn get_issue(&self, oid_or_id: &str) -> Result<Issue> {
        let index = read_index(self.repo, ISSUE_INDEX)?;
        let known_oids = self.repo.list(ISSUE_PREFIX)?;
        let oid = resolve_oid(index.as_ref(), &known_oids, oid_or_id)?;
        let ref_name = format!("{ISSUE_PREFIX}{oid}");
        let entry = self.repo.read(&ref_name)?;
        let display_id = display_id_for_oid(index.as_ref(), &oid);
        issue_from_entry(&entry, display_id)
    }

    /// List all issues in the repository.
    ///
    /// # Errors
    /// Returns an error if any git operation fails.
    pub fn list_issues(&self) -> Result<Vec<Issue>> {
        let index = read_index(self.repo, ISSUE_INDEX)?;
        let oids = self.repo.list(ISSUE_PREFIX)?;
        oids.into_iter()
            .map(|oid| {
                let ref_name = format!("{ISSUE_PREFIX}{oid}");
                let entry = self.repo.read(&ref_name)?;
                let display_id = display_id_for_oid(index.as_ref(), &oid);
                issue_from_entry(&entry, display_id)
            })
            .collect()
    }

    /// List issues filtered by state.
    ///
    /// # Errors
    /// Returns an error if any git operation fails.
    pub fn list_issues_by_state(&self, state: &IssueState) -> Result<Vec<Issue>> {
        Ok(self
            .list_issues()?
            .into_iter()
            .filter(|i| &i.state == state)
            .collect())
    }

    /// Rebuild the issue display-ID index from scratch.
    ///
    /// Reads every issue's `source/url` field, matches it against the current
    /// config sigils, and writes new `{sigil}{number}` → OID entries. Locally
    /// created issues (no `source/url`) retain their existing display ID if one
    /// is present, or are assigned a sequential numeric ID.
    ///
    /// Returns the number of entries written.
    ///
    /// # Errors
    /// Returns an error if any git operation fails.
    pub fn reindex_issues(&self) -> Result<usize> {
        use crate::refs;

        let old_index = read_index(self.repo, ISSUE_INDEX)?;
        let oids = self.repo.list(ISSUE_PREFIX)?;

        // Build (owner, repo) → issue sigil from config.
        let sigil_map = crate::reindex::build_sigil_map(self.repo, "issue")?;

        let mut entries: Vec<(String, String)> = Vec::new();
        let mut next_local_id = 1u64;

        for oid in &oids {
            let ref_name = format!("{ISSUE_PREFIX}{oid}");
            let entry = self.repo.read(&ref_name)?;

            if let Some(display_id) =
                crate::reindex::display_id_from_source(&entry, &sigil_map, "issues")
            {
                entries.push((display_id, oid.clone()));
            } else {
                // Local issue — keep existing display ID or assign next sequential.
                let existing = old_index
                    .as_ref()
                    .and_then(|idx| idx.iter().find(|(_, v)| v.as_str() == oid))
                    .map(|(k, _)| k.clone());
                let display_id = existing.unwrap_or_else(|| {
                    let id = next_local_id.to_string();
                    next_local_id += 1;
                    id
                });
                entries.push((display_id, oid.clone()));
            }
        }

        let count = entries.len();
        let pairs: Vec<(&str, &str)> = entries
            .iter()
            .map(|(k, v)| (k.as_str(), v.as_str()))
            .collect();
        crate::reindex::write_index_from_scratch(self.repo, refs::ISSUE_INDEX, &pairs)?;
        Ok(count)
    }

    /// Apply a partial update to an issue.
    ///
    /// # Errors
    /// Returns [`Error::NotFound`] if the issue does not exist, or a git error on failure.
    #[allow(clippy::too_many_arguments)]
    pub fn update_issue(
        &self,
        oid_or_id: &str,
        title: Option<&str>,
        body: Option<&str>,
        state: Option<&IssueState>,
        add_labels: &[&str],
        remove_labels: &[&str],
        add_assignees: &[&str],
        remove_assignees: &[&str],
        source_url: Option<&str>,
    ) -> Result<Issue> {
        let index = read_index(self.repo, ISSUE_INDEX)?;
        let known_oids = self.repo.list(ISSUE_PREFIX)?;
        let oid = resolve_oid(index.as_ref(), &known_oids, oid_or_id)?;
        let ref_name = format!("{ISSUE_PREFIX}{oid}");

        let add_label_keys: Vec<String> =
            add_labels.iter().map(|l| format!("labels/{l}")).collect();
        let rem_label_keys: Vec<String> = remove_labels
            .iter()
            .map(|l| format!("labels/{l}"))
            .collect();
        let add_assignee_keys: Vec<String> = add_assignees
            .iter()
            .map(|a| format!("assignees/{a}"))
            .collect();
        let rem_assignee_keys: Vec<String> = remove_assignees
            .iter()
            .map(|a| format!("assignees/{a}"))
            .collect();
        let state_bytes: Option<String> = state.map(|s| s.as_str().to_string());

        let mut mutations: Vec<Mutation<'_>> = Vec::new();
        if let Some(t) = title {
            mutations.push(Mutation::Set("title", t.as_bytes()));
        }
        if let Some(b) = body {
            mutations.push(Mutation::Set("body", b.as_bytes()));
        }
        if let Some(ref s) = state_bytes {
            mutations.push(Mutation::Set("state", s.as_bytes()));
        }
        for k in &add_label_keys {
            mutations.push(Mutation::Set(k.as_str(), b""));
        }
        for k in &rem_label_keys {
            mutations.push(Mutation::Delete(k.as_str()));
        }
        for k in &add_assignee_keys {
            mutations.push(Mutation::Set(k.as_str(), b""));
        }
        for k in &rem_assignee_keys {
            mutations.push(Mutation::Delete(k.as_str()));
        }
        if let Some(url) = source_url {
            mutations.push(Mutation::Set("source/url", url.as_bytes()));
        }

        let entry = self.repo.update(&ref_name, &mutations, "update issue")?;
        let display_id = display_id_for_oid(index.as_ref(), &oid);
        issue_from_entry(&entry, display_id)
    }
}
