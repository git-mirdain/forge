//! MCP tool definitions for forge comments (v2 API).

use rmcp::handler::server::wrapper::Parameters;
use rmcp::{tool, tool_router};
use schemars::JsonSchema;
use serde::Deserialize;

use git_forge::comment::{
    Anchor, create_thread, edit_in_thread, list_thread_comments, reply_to_thread, resolve_thread,
};
use git_forge::exe::Executor;

use crate::server::ForgeMcpServer;

/// Parameters for the `list_comments_on` tool.
#[derive(Deserialize, JsonSchema)]
struct ListCommentsOnParams {
    /// Git object OID (blob, commit, tree) to list comments on.
    oid: String,
}

/// Parameters for the `create_comment` tool.
#[derive(Deserialize, JsonSchema)]
struct CreateCommentParams {
    /// Comment body (Markdown).
    body: String,
    /// Git object OID to anchor the comment to.
    anchor_oid: String,
    /// Optional start line (1-based).
    start_line: Option<u32>,
    /// Optional end line (1-based).
    end_line: Option<u32>,
}

/// Parameters for the `reply_comment` tool.
#[derive(Deserialize, JsonSchema)]
struct ReplyCommentParams {
    /// Thread UUID.
    thread_id: String,
    /// OID of the comment to reply to.
    reply_to_oid: String,
    /// Reply body (Markdown).
    body: String,
}

/// Parameters for the `resolve_comment` tool.
#[derive(Deserialize, JsonSchema)]
struct ResolveCommentParams {
    /// Thread UUID.
    thread_id: String,
    /// OID of the comment that roots the thread.
    comment_oid: String,
    /// Optional resolution message.
    message: Option<String>,
}

/// Parameters for the `show_thread` tool.
#[derive(Deserialize, JsonSchema)]
struct ShowThreadParams {
    /// Thread UUID.
    thread_id: String,
}

/// Parameters for the `edit_comment_in_thread` tool.
#[derive(Deserialize, JsonSchema)]
struct EditCommentParams {
    /// Thread UUID.
    thread_id: String,
    /// OID of the comment to edit.
    comment_oid: String,
    /// New body (Markdown).
    body: String,
}

#[tool_router(router = comment_router, vis = "pub(crate)")]
impl ForgeMcpServer {
    /// List all comment threads anchored to a git object (blob, commit, or tree).
    #[tool(description = "List all comments anchored to a git object OID.")]
    fn list_comments_on(&self, Parameters(params): Parameters<ListCommentsOnParams>) -> String {
        let repo = match self.open_repo() {
            Ok(r) => r,
            Err(e) => return format!("error: {e}"),
        };
        let workdir = match repo.workdir() {
            Some(p) => p.to_path_buf(),
            None => repo.path().to_path_buf(),
        };
        let exec = match Executor::from_path(&workdir) {
            Ok(e) => e,
            Err(e) => return format!("error: {e}"),
        };
        match exec.list_comments_on(&params.oid) {
            Ok(comments) => facet_json::to_string_pretty(&comments).expect("serialize"),
            Err(e) => format!("error: {e}"),
        }
    }

    /// Create a new comment thread anchored to a git object.
    #[tool(description = "Create a new comment thread anchored to a git object OID.")]
    fn create_comment(&self, Parameters(params): Parameters<CreateCommentParams>) -> String {
        let repo = match self.open_repo() {
            Ok(r) => r,
            Err(e) => return format!("error: {e}"),
        };
        let anchor = Anchor {
            oid: params.anchor_oid,
            start_line: params.start_line,
            end_line: params.end_line,
        };
        match create_thread(&repo, &params.body, Some(&anchor), None) {
            Ok((thread_id, comment)) => {
                let comment_json = facet_json::to_string_pretty(&comment).expect("serialize");
                format!("{{\"thread_id\":{thread_id:?},\"comment\":{comment_json}}}")
            }
            Err(e) => format!("error: {e}"),
        }
    }

    /// Reply to an existing comment thread.
    #[tool(description = "Append a reply to an existing comment thread.")]
    fn reply_comment(&self, Parameters(params): Parameters<ReplyCommentParams>) -> String {
        let repo = match self.open_repo() {
            Ok(r) => r,
            Err(e) => return format!("error: {e}"),
        };
        match reply_to_thread(
            &repo,
            &params.thread_id,
            &params.body,
            &params.reply_to_oid,
            None,
            None,
        ) {
            Ok(comment) => facet_json::to_string_pretty(&comment).expect("serialize"),
            Err(e) => format!("error: {e}"),
        }
    }

    /// Resolve a comment thread.
    #[tool(description = "Resolve a comment thread.")]
    fn resolve_comment(&self, Parameters(params): Parameters<ResolveCommentParams>) -> String {
        let repo = match self.open_repo() {
            Ok(r) => r,
            Err(e) => return format!("error: {e}"),
        };
        match resolve_thread(
            &repo,
            &params.thread_id,
            &params.comment_oid,
            params.message.as_deref(),
        ) {
            Ok(comment) => facet_json::to_string_pretty(&comment).expect("serialize"),
            Err(e) => format!("error: {e}"),
        }
    }

    /// Show all comments in a thread.
    #[tool(description = "Show all comments in a thread, identified by thread UUID.")]
    fn show_thread(&self, Parameters(params): Parameters<ShowThreadParams>) -> String {
        let repo = match self.open_repo() {
            Ok(r) => r,
            Err(e) => return format!("error: {e}"),
        };
        match list_thread_comments(&repo, &params.thread_id) {
            Ok(comments) => facet_json::to_string_pretty(&comments).expect("serialize"),
            Err(e) => format!("error: {e}"),
        }
    }

    /// Edit a comment in a thread.
    #[tool(description = "Edit a comment in a thread.")]
    fn edit_comment_in_thread(&self, Parameters(params): Parameters<EditCommentParams>) -> String {
        let repo = match self.open_repo() {
            Ok(r) => r,
            Err(e) => return format!("error: {e}"),
        };
        match edit_in_thread(
            &repo,
            &params.thread_id,
            &params.comment_oid,
            &params.body,
            None,
            None,
        ) {
            Ok(comment) => facet_json::to_string_pretty(&comment).expect("serialize"),
            Err(e) => format!("error: {e}"),
        }
    }
}
