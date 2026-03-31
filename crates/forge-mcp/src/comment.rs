//! MCP tool definitions for forge issue comments.
// Uses v1 comment functions temporarily until Phase 9 MCP update.
#![allow(deprecated)]

use rmcp::handler::server::wrapper::Parameters;
use rmcp::{tool, tool_router};
use schemars::JsonSchema;
use serde::Deserialize;

use git_forge::Store;
use git_forge::comment::{issue_comment_ref, list_comments};

use crate::server::ForgeMcpServer;

/// Parameters for the `list_issue_comments` tool.
#[derive(Deserialize, JsonSchema)]
struct ListIssueCommentsParams {
    /// Issue display ID (e.g. `"3"`, `"GH1"`) or OID prefix.
    reference: String,
}

#[tool_router(router = comment_router, vis = "pub(crate)")]
impl ForgeMcpServer {
    /// List comments for an issue.
    #[tool(description = "List comments for an issue, identified by display ID or OID prefix.")]
    fn list_issue_comments(
        &self,
        Parameters(params): Parameters<ListIssueCommentsParams>,
    ) -> String {
        let repo = match self.open_repo() {
            Ok(r) => r,
            Err(e) => return format!("error: {e}"),
        };
        let store = Store::new(&repo);
        let issue = match store.get_issue(&params.reference) {
            Ok(i) => i,
            Err(e) => return format!("error: {e}"),
        };
        let ref_name = issue_comment_ref(&issue.oid);
        match list_comments(&repo, &ref_name) {
            Ok(comments) => facet_json::to_string_pretty(&comments).expect("serialize"),
            Err(e) => format!("error: {e}"),
        }
    }
}
