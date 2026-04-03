//! MCP tool definitions for forge reviews and review comments.

use rmcp::handler::server::wrapper::Parameters;
use rmcp::{tool, tool_router};
use schemars::JsonSchema;
use serde::Deserialize;

use git_forge::Store;
use git_forge::comment::{find_threads_by_object, list_thread_comments};
use git_forge::refs::walk_tree;
use git_forge::review::{ReviewState, ReviewTarget};

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

/// Parameters for the `create_review` tool.
#[derive(Deserialize, JsonSchema)]
struct CreateReviewParams {
    /// Review title.
    title: String,
    /// Head object OID (commit, tree, or blob).
    head: String,
    /// Base object OID (optional; makes this a commit-range review).
    base: Option<String>,
    /// Body in Markdown.
    body: Option<String>,
    /// Source ref name to track (e.g. `"feature-branch"`).
    source_ref: Option<String>,
}

/// Parameters for the `approve_review` tool.
#[derive(Deserialize, JsonSchema)]
struct ApproveReviewParams {
    /// Review display ID or OID prefix.
    reference: String,
    /// Contributor UUID to record as the approver.
    contributor_uuid: String,
}

/// Parameters for the `update_review` tool.
#[derive(Deserialize, JsonSchema)]
struct UpdateReviewParams {
    /// Review display ID or OID prefix.
    reference: String,
    /// New title.
    title: Option<String>,
    /// New body (Markdown).
    body: Option<String>,
    /// New state: `"open"`, `"draft"`, `"closed"`, or `"merged"`.
    state: Option<String>,
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

    /// Create a new review.
    #[tool(
        description = "Create a new forge review targeting a git object (commit, blob, or tree)."
    )]
    fn create_review(&self, Parameters(params): Parameters<CreateReviewParams>) -> String {
        let repo = match self.open_repo() {
            Ok(r) => r,
            Err(e) => return format!("error: {e}"),
        };
        let store = Store::new(&repo);
        let target = ReviewTarget {
            head: params.head,
            base: params.base,
        };
        match store.create_review(
            &params.title,
            params.body.as_deref().unwrap_or(""),
            &target,
            params.source_ref.as_deref(),
        ) {
            Ok(review) => facet_json::to_string_pretty(&review).expect("serialize"),
            Err(e) => format!("error: {e}"),
        }
    }

    /// Approve all objects in a review for a contributor.
    #[tool(description = "Record approval of all objects in a review for a contributor UUID.")]
    fn approve_review(&self, Parameters(params): Parameters<ApproveReviewParams>) -> String {
        let repo = match self.open_repo() {
            Ok(r) => r,
            Err(e) => return format!("error: {e}"),
        };
        let store = Store::new(&repo);
        match store.approve_review(&params.reference, &params.contributor_uuid) {
            Ok(review) => facet_json::to_string_pretty(&review).expect("serialize"),
            Err(e) => format!("error: {e}"),
        }
    }

    /// Update an existing review.
    #[tool(
        description = "Update an existing forge review by display ID or OID prefix. All fields are optional."
    )]
    fn update_review(&self, Parameters(params): Parameters<UpdateReviewParams>) -> String {
        let repo = match self.open_repo() {
            Ok(r) => r,
            Err(e) => return format!("error: {e}"),
        };
        let store = Store::new(&repo);
        let state = match params.state.as_deref() {
            None => None,
            Some(s) => match s.parse::<ReviewState>() {
                Ok(st) => Some(st),
                Err(_) => {
                    return format!("error: {}", git_forge::Error::InvalidState(s.to_string()));
                }
            },
        };
        match store.update_review(
            &params.reference,
            params.title.as_deref(),
            params.body.as_deref(),
            state.as_ref(),
        ) {
            Ok(review) => facet_json::to_string_pretty(&review).expect("serialize"),
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

    fn test_server_with_head() -> (TempDir, ForgeMcpServer, String) {
        let dir = TempDir::new().expect("temp dir");
        let repo = Repository::init(dir.path()).expect("init repo");
        {
            let mut cfg = repo.config().expect("config");
            cfg.set_str("user.name", "test").expect("user.name");
            cfg.set_str("user.email", "test@test.com")
                .expect("user.email");
        }
        let sig = git2::Signature::now("test", "test@test.com").expect("sig");
        let mut index = repo.index().expect("index");
        let tree_oid = index.write_tree().expect("write tree");
        let tree = repo.find_tree(tree_oid).expect("find tree");
        let commit_oid = repo
            .commit(Some("HEAD"), &sig, &sig, "initial", &tree, &[])
            .expect("initial commit");
        let git_dir = repo.path().to_path_buf();
        let server = ForgeMcpServer::for_test(git_dir);
        (dir, server, commit_oid.to_string())
    }

    #[test]
    fn update_review_changes_state() {
        let (_dir, server, head) = test_server_with_head();
        let created = server.create_review(Parameters(super::CreateReviewParams {
            title: "my review".to_string(),
            head,
            base: None,
            body: None,
            source_ref: None,
        }));
        assert!(!created.starts_with("error:"), "create failed: {created}");

        let oid = {
            let v: serde_json::Value = serde_json::from_str(&created).expect("parse");
            v["oid"].as_str().expect("oid").to_string()
        };

        let updated = server.update_review(Parameters(super::UpdateReviewParams {
            reference: oid,
            title: Some("updated title".to_string()),
            body: None,
            state: Some("closed".to_string()),
        }));
        assert!(!updated.starts_with("error:"), "update failed: {updated}");
        assert!(updated.contains("updated title"));
        assert!(updated.contains("Closed"));
    }

    #[test]
    fn update_review_invalid_state_returns_error() {
        let (_dir, server, head) = test_server_with_head();
        let created = server.create_review(Parameters(super::CreateReviewParams {
            title: "state test".to_string(),
            head,
            base: None,
            body: None,
            source_ref: None,
        }));
        let oid = {
            let v: serde_json::Value = serde_json::from_str(&created).expect("parse");
            v["oid"].as_str().expect("oid").to_string()
        };
        let result = server.update_review(Parameters(super::UpdateReviewParams {
            reference: oid,
            title: None,
            body: None,
            state: Some("bogus".to_string()),
        }));
        assert!(result.starts_with("error:"));
    }
}
