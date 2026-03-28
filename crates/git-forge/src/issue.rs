//! Issue entity CRUD backed by `git-ledger`.

use facet::Facet;
use git_ledger::{IdStrategy, Ledger, LedgerEntry, Mutation};
use serde::Serialize;

use crate::index::{index_upsert, read_index, resolve_oid};
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

    let display_id = match display_id.as_deref() {
        Some("pending") | None => None,
        Some(_) => display_id,
    };

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
        let mut fields: Vec<(&str, &[u8])> = vec![
            ("title", title.as_bytes()),
            ("state", b"open"),
            ("body", body.as_bytes()),
        ];

        let label_paths: Vec<String> = labels.iter().map(|l| format!("labels/{l}")).collect();
        let assignee_paths: Vec<String> =
            assignees.iter().map(|a| format!("assignees/{a}")).collect();
        let label_fields: Vec<(&str, &[u8])> = label_paths
            .iter()
            .map(|p| (p.as_str(), b"" as &[u8]))
            .collect();
        let assignee_fields: Vec<(&str, &[u8])> = assignee_paths
            .iter()
            .map(|p| (p.as_str(), b"" as &[u8]))
            .collect();
        fields.extend(label_fields);
        fields.extend(assignee_fields);

        let entry = self.repo.create(
            ISSUE_PREFIX,
            &IdStrategy::CommitOid,
            &fields,
            "create issue",
            None,
        )?;

        index_upsert(self.repo, ISSUE_INDEX, &[(&entry.id, "pending")])?;

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
        let mut fields: Vec<(&str, &[u8])> = vec![
            ("title", title.as_bytes()),
            ("state", b"open"),
            ("body", body.as_bytes()),
            ("source/url", source.as_bytes()),
        ];

        let label_paths: Vec<String> = labels.iter().map(|l| format!("labels/{l}")).collect();
        let assignee_paths: Vec<String> =
            assignees.iter().map(|a| format!("assignees/{a}")).collect();
        let label_fields: Vec<(&str, &[u8])> = label_paths
            .iter()
            .map(|p| (p.as_str(), b"" as &[u8]))
            .collect();
        let assignee_fields: Vec<(&str, &[u8])> = assignee_paths
            .iter()
            .map(|p| (p.as_str(), b"" as &[u8]))
            .collect();
        fields.extend(label_fields);
        fields.extend(assignee_fields);

        let entry = self.repo.create(
            ISSUE_PREFIX,
            &IdStrategy::CommitOid,
            &fields,
            "forge: create issue",
            Some(author),
        )?;

        index_upsert(
            self.repo,
            ISSUE_INDEX,
            &[(&entry.id, &entry.id), (display_id, &entry.id)],
        )?;

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
        let oid = resolve_oid(index.as_ref(), oid_or_id)?;
        let ref_name = format!("{ISSUE_PREFIX}{oid}");
        let entry = self.repo.read(&ref_name)?;
        let display_id = index.as_ref().and_then(|m| m.get(&oid)).cloned();
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
                let display_id = index.as_ref().and_then(|m| m.get(&oid)).cloned();
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
    ) -> Result<Issue> {
        let index = read_index(self.repo, ISSUE_INDEX)?;
        let oid = resolve_oid(index.as_ref(), oid_or_id)?;
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

        let entry = self.repo.update(&ref_name, &mutations, "update issue")?;
        let display_id = index.as_ref().and_then(|m| m.get(&oid)).cloned();
        issue_from_entry(&entry, display_id)
    }
}
