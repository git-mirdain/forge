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
}
