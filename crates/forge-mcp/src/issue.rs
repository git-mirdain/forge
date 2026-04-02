//! MCP tool definitions for forge issues.

use rmcp::handler::server::wrapper::Parameters;
use rmcp::{tool, tool_router};
use schemars::JsonSchema;
use serde::Deserialize;

use git_forge::Store;
use git_forge::issue::IssueState;

use crate::server::ForgeMcpServer;

/// Parameters for the `list_issues` tool.
#[derive(Deserialize, JsonSchema)]
struct ListIssuesParams {
    /// Filter by state: `"open"` or `"closed"`. Omit to return all issues.
    state: Option<String>,
}

/// Parameters for the `get_issue` tool.
#[derive(Deserialize, JsonSchema)]
struct GetIssueParams {
    /// Display ID (e.g. `"3"`, `"GH1"`) or OID prefix.
    reference: String,
}

/// Parameters for the `create_issue` tool.
#[derive(Deserialize, JsonSchema)]
struct CreateIssueParams {
    /// Issue title.
    title: String,
    /// Issue body (Markdown). Defaults to empty.
    body: Option<String>,
    /// Labels to attach.
    labels: Option<Vec<String>>,
    /// Contributor UUIDs to assign.
    assignees: Option<Vec<String>>,
}

/// Parameters for the `update_issue` tool.
#[derive(Deserialize, JsonSchema)]
struct UpdateIssueParams {
    /// Display ID (e.g. `"3"`, `"GH1"`) or OID prefix.
    reference: String,
    /// New title.
    title: Option<String>,
    /// New body (Markdown).
    body: Option<String>,
    /// New state: `"open"` or `"closed"`.
    state: Option<String>,
    /// Labels to add.
    add_labels: Option<Vec<String>>,
    /// Labels to remove.
    remove_labels: Option<Vec<String>>,
    /// Contributor UUIDs to assign.
    add_assignees: Option<Vec<String>>,
    /// Contributor UUIDs to unassign.
    remove_assignees: Option<Vec<String>>,
}

#[tool_router(router = issue_router, vis = "pub(crate)")]
impl ForgeMcpServer {
    /// List issues in the forge repository.
    #[tool(description = "List issues in the forge repository.")]
    fn list_issues(&self, Parameters(params): Parameters<ListIssuesParams>) -> String {
        let repo = match self.open_repo() {
            Ok(r) => r,
            Err(e) => return format!("error: {e}"),
        };
        let store = Store::new(&repo);
        let issues = match params.state.as_deref() {
            None => store.list_issues(),
            Some(s) => s.parse::<IssueState>().map_or_else(
                |_| Err(git_forge::Error::InvalidState(s.to_string())),
                |state| store.list_issues_by_state(&state),
            ),
        };
        match issues {
            Ok(list) => facet_json::to_string_pretty(&list).expect("serialize"),
            Err(e) => format!("error: {e}"),
        }
    }

    /// Get a single issue by display ID or OID prefix.
    #[tool(description = "Get a single issue by display ID (e.g. \"3\", \"GH1\") or OID prefix.")]
    fn get_issue(&self, Parameters(params): Parameters<GetIssueParams>) -> String {
        let repo = match self.open_repo() {
            Ok(r) => r,
            Err(e) => return format!("error: {e}"),
        };
        let store = Store::new(&repo);
        match store.get_issue(&params.reference) {
            Ok(issue) => facet_json::to_string_pretty(&issue).expect("serialize"),
            Err(e) => format!("error: {e}"),
        }
    }

    /// Create a new issue.
    #[tool(description = "Create a new forge issue.")]
    fn create_issue(&self, Parameters(params): Parameters<CreateIssueParams>) -> String {
        let repo = match self.open_repo() {
            Ok(r) => r,
            Err(e) => return format!("error: {e}"),
        };
        let store = Store::new(&repo);
        let labels: Vec<&str> = params
            .labels
            .as_deref()
            .unwrap_or(&[])
            .iter()
            .map(String::as_str)
            .collect();
        let assignees: Vec<&str> = params
            .assignees
            .as_deref()
            .unwrap_or(&[])
            .iter()
            .map(String::as_str)
            .collect();
        match store.create_issue(
            &params.title,
            params.body.as_deref().unwrap_or(""),
            &labels,
            &assignees,
        ) {
            Ok(issue) => facet_json::to_string_pretty(&issue).expect("serialize"),
            Err(e) => format!("error: {e}"),
        }
    }

    /// Update an existing issue.
    #[tool(
        description = "Update an existing forge issue by display ID or OID prefix. All fields are optional."
    )]
    fn update_issue(&self, Parameters(params): Parameters<UpdateIssueParams>) -> String {
        let repo = match self.open_repo() {
            Ok(r) => r,
            Err(e) => return format!("error: {e}"),
        };
        let store = Store::new(&repo);
        let state = match params.state.as_deref() {
            None => None,
            Some(s) => match s.parse::<IssueState>() {
                Ok(st) => Some(st),
                Err(_) => {
                    return format!("error: {}", git_forge::Error::InvalidState(s.to_string()));
                }
            },
        };
        let add_labels: Vec<&str> = params
            .add_labels
            .as_deref()
            .unwrap_or(&[])
            .iter()
            .map(String::as_str)
            .collect();
        let remove_labels: Vec<&str> = params
            .remove_labels
            .as_deref()
            .unwrap_or(&[])
            .iter()
            .map(String::as_str)
            .collect();
        let add_assignees: Vec<&str> = params
            .add_assignees
            .as_deref()
            .unwrap_or(&[])
            .iter()
            .map(String::as_str)
            .collect();
        let remove_assignees: Vec<&str> = params
            .remove_assignees
            .as_deref()
            .unwrap_or(&[])
            .iter()
            .map(String::as_str)
            .collect();
        match store.update_issue(
            &params.reference,
            params.title.as_deref(),
            params.body.as_deref(),
            state.as_ref(),
            &add_labels,
            &remove_labels,
            &add_assignees,
            &remove_assignees,
        ) {
            Ok(issue) => facet_json::to_string_pretty(&issue).expect("serialize"),
            Err(e) => format!("error: {e}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use git2::Repository;
    use rmcp::handler::server::wrapper::Parameters;
    use tempfile::TempDir;

    use crate::server::ForgeMcpServer;

    fn test_server() -> (TempDir, ForgeMcpServer) {
        let dir = TempDir::new().expect("temp dir");
        let repo = Repository::init(dir.path()).expect("init repo");
        {
            let mut cfg = repo.config().expect("config");
            cfg.set_str("user.name", "test").expect("user.name");
            cfg.set_str("user.email", "test@test.com")
                .expect("user.email");
        }
        {
            let sig = git2::Signature::now("test", "test@test.com").expect("sig");
            let mut index = repo.index().expect("index");
            let tree_oid = index.write_tree().expect("write tree");
            let tree = repo.find_tree(tree_oid).expect("find tree");
            repo.commit(Some("HEAD"), &sig, &sig, "initial", &tree, &[])
                .expect("initial commit");
        }
        let git_dir = repo.path().to_path_buf();
        let server = ForgeMcpServer::for_test(git_dir);
        (dir, server)
    }

    #[test]
    fn create_issue_returns_json() {
        let (_dir, server) = test_server();
        let result = server.create_issue(Parameters(super::CreateIssueParams {
            title: "test issue".to_string(),
            body: Some("body text".to_string()),
            labels: None,
            assignees: None,
        }));
        assert!(!result.starts_with("error:"), "got error: {result}");
        assert!(result.contains("test issue"));
    }

    #[test]
    fn update_issue_changes_state() {
        let (_dir, server) = test_server();
        let created = server.create_issue(Parameters(super::CreateIssueParams {
            title: "close me".to_string(),
            body: None,
            labels: None,
            assignees: None,
        }));
        assert!(!created.starts_with("error:"), "create failed: {created}");

        let oid = {
            let v: serde_json::Value = serde_json::from_str(&created).expect("parse");
            v["oid"].as_str().expect("oid field").to_string()
        };

        let updated = server.update_issue(Parameters(super::UpdateIssueParams {
            reference: oid,
            title: None,
            body: None,
            state: Some("closed".to_string()),
            add_labels: None,
            remove_labels: None,
            add_assignees: None,
            remove_assignees: None,
        }));
        assert!(!updated.starts_with("error:"), "update failed: {updated}");
        assert!(updated.contains("Closed"));
    }

    #[test]
    fn update_issue_invalid_state_returns_error() {
        let (_dir, server) = test_server();
        let created = server.create_issue(Parameters(super::CreateIssueParams {
            title: "state test".to_string(),
            body: None,
            labels: None,
            assignees: None,
        }));
        let oid = {
            let v: serde_json::Value = serde_json::from_str(&created).expect("parse");
            v["oid"].as_str().expect("oid").to_string()
        };
        let result = server.update_issue(Parameters(super::UpdateIssueParams {
            reference: oid,
            title: None,
            body: None,
            state: Some("bogus".to_string()),
            add_labels: None,
            remove_labels: None,
            add_assignees: None,
            remove_assignees: None,
        }));
        assert!(result.starts_with("error:"));
    }
}
