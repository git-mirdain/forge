//! MCP tool definitions for forge reviews and review comments.

use rmcp::handler::server::wrapper::Parameters;
use rmcp::{tool, tool_router};
use schemars::JsonSchema;
use serde::Deserialize;

use git_forge::Store;
use git_forge::comment::{find_threads_by_object, list_thread_comments};
use git_forge::refs::walk_tree;
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

        let mut comments = Vec::new();

        // Collect blob-anchored comments via v2 thread discovery.
        if let Ok(oid) = git2::Oid::from_str(&review.target.head)
            && let Ok(obj) = repo.find_object(oid, None)
        {
            let mut files = Vec::new();
            match obj.kind() {
                Some(git2::ObjectType::Blob) => {
                    files.push(("(blob)".to_string(), review.target.head.clone()));
                }
                Some(git2::ObjectType::Tree) => {
                    if let Ok(tree) = repo.find_tree(oid) {
                        walk_tree(&repo, &tree, "", &mut files);
                    }
                }
                Some(git2::ObjectType::Commit) => {
                    if let Ok(commit) = repo.find_commit(oid)
                        && let Ok(tree) = commit.tree()
                    {
                        walk_tree(&repo, &tree, "", &mut files);
                    }
                }
                _ => {}
            }
            let mut seen = std::collections::HashSet::new();
            for (_, blob_oid) in &files {
                if !seen.insert(blob_oid.clone()) {
                    continue;
                }
                if let Ok(thread_ids) = find_threads_by_object(&repo, blob_oid) {
                    for tid in &thread_ids {
                        if let Ok(cs) = list_thread_comments(&repo, tid) {
                            comments.extend(cs);
                        }
                    }
                }
            }
        }

        // Also collect threads anchored to the review OID itself.
        if let Ok(thread_ids) = find_threads_by_object(&repo, &review.oid) {
            for tid in &thread_ids {
                if let Ok(cs) = list_thread_comments(&repo, tid) {
                    comments.extend(cs);
                }
            }
        }

        comments.sort_by_key(|c| c.timestamp);
        facet_json::to_string_pretty(&comments).expect("serialize")
    }
}
