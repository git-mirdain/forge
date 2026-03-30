//! MCP tool definitions for forge reviews and review comments.

use rmcp::handler::server::wrapper::Parameters;
use rmcp::{tool, tool_router};
use schemars::JsonSchema;
use serde::Deserialize;

use git_forge::Store;
use git_forge::comment::{list_comments, review_comment_ref};
use git_forge::review::ReviewState;

use crate::server::ForgeMcpServer;

/// Parameters for the `list_reviews` tool.
#[derive(Deserialize, JsonSchema)]
struct ListReviewsParams {
    /// Filter by state: `"open"` or `"closed"`. Omit to return all reviews.
    state: Option<String>,
}

/// Parameters for the `get_review` tool.
#[derive(Deserialize, JsonSchema)]
struct GetReviewParams {
    /// Display ID (e.g. `"GH#1"`) or OID prefix.
    reference: String,
}

/// Parameters for the `list_review_comments` tool.
#[derive(Deserialize, JsonSchema)]
struct ListReviewCommentsParams {
    /// Review display ID or OID prefix.
    reference: String,
}

#[tool_router(router = review_router, vis = "pub(crate)")]
impl ForgeMcpServer {
    /// List reviews in the forge repository.
    #[tool(description = "List reviews in the forge repository.")]
    fn list_reviews(&self, Parameters(params): Parameters<ListReviewsParams>) -> String {
        let repo = match self.open_repo() {
            Ok(r) => r,
            Err(e) => return format!("error: {e}"),
        };
        let store = Store::new(&repo);
        let reviews = match params.state.as_deref() {
            None => store.list_reviews(),
            Some(s) => s.parse::<ReviewState>().map_or_else(
                |_| Err(git_forge::Error::InvalidState(s.to_string())),
                |state| store.list_reviews_by_state(&state),
            ),
        };
        match reviews {
            Ok(list) => facet_json::to_string_pretty(&list).expect("serialize"),
            Err(e) => format!("error: {e}"),
        }
    }

    /// Get a single review by display ID or OID prefix.
    #[tool(description = "Get a single review by display ID (e.g. \"GH#1\") or OID prefix.")]
    fn get_review(&self, Parameters(params): Parameters<GetReviewParams>) -> String {
        let repo = match self.open_repo() {
            Ok(r) => r,
            Err(e) => return format!("error: {e}"),
        };
        let store = Store::new(&repo);
        match store.get_review(&params.reference) {
            Ok(review) => facet_json::to_string_pretty(&review).expect("serialize"),
            Err(e) => format!("error: {e}"),
        }
    }

    /// List comments for a review.
    #[tool(description = "List comments for a review, identified by display ID or OID prefix.")]
    fn list_review_comments(
        &self,
        Parameters(params): Parameters<ListReviewCommentsParams>,
    ) -> String {
        let repo = match self.open_repo() {
            Ok(r) => r,
            Err(e) => return format!("error: {e}"),
        };
        let store = Store::new(&repo);
        let review = match store.get_review(&params.reference) {
            Ok(r) => r,
            Err(e) => return format!("error: {e}"),
        };
        let ref_name = review_comment_ref(&review.oid);
        match list_comments(&repo, &ref_name) {
            Ok(comments) => facet_json::to_string_pretty(&comments).expect("serialize"),
            Err(e) => format!("error: {e}"),
        }
    }
}
