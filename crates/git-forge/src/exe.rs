//! Executor for forge commands.
//!
//! [`Executor`] owns a [`git2::Repository`] and exposes typed methods for each
//! forge operation. The `run` method (available with the `cli` feature) dispatches
//! from a parsed [`crate::cli::Cli`] and writes output to stdout.

use std::collections::BTreeMap;
use std::io::IsTerminal;
use std::path::Path;

use facet::Facet;
use git2::{ErrorCode, ObjectType, Repository};
use serde::Serialize;

use crate::cli::CommentStateFilter;
use crate::comment::{
    Anchor, Comment, create_thread, find_threads_by_object, list_all_thread_ids,
    list_thread_comments, reply_to_thread, resolve_thread, thread_is_resolved,
};
use crate::issue::{Issue, IssueState};
use crate::refs::walk_tree;
use crate::review::{Review, ReviewState, ReviewTarget};
use crate::{Error, Result, Store};

/// A provider configuration entry for JSON serialization.
#[derive(Facet)]
pub struct ConfigEntry {
    provider: String,
    owner: String,
    repo: String,
    sigils: BTreeMap<String, String>,
}

/// Owns a [`Repository`] and executes forge operations.
pub struct Executor {
    repo: Repository,
}

impl Executor {
    /// Discover the repository from the current directory.
    ///
    /// # Errors
    /// Returns an error if no git repository can be found.
    pub fn discover() -> Result<Self> {
        Ok(Self {
            repo: Repository::discover(".")?,
        })
    }

    /// Open the repository at `path`.
    ///
    /// # Errors
    /// Returns an error if the path is not a git repository.
    pub fn from_path(path: &Path) -> Result<Self> {
        Ok(Self {
            repo: Repository::open(path)?,
        })
    }

    fn store(&self) -> Store<'_> {
        Store::new(&self.repo)
    }

    /// Create a new issue.
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
        self.store().create_issue(title, body, labels, assignees)
    }

    /// Fetch an issue by display ID or OID prefix.
    ///
    /// # Errors
    /// Returns [`crate::Error::NotFound`] if no matching issue exists.
    pub fn get_issue(&self, reference: &str) -> Result<Issue> {
        self.store().get_issue(reference)
    }

    /// List all issues, optionally filtered by state.
    ///
    /// # Errors
    /// Returns an error if a git operation fails.
    pub fn list_issues(&self, state: Option<&IssueState>) -> Result<Vec<Issue>> {
        match state {
            Some(s) => self.store().list_issues_by_state(s),
            None => self.store().list_issues(),
        }
    }

    /// Apply a partial update to an issue.
    ///
    /// # Errors
    /// Returns [`crate::Error::NotFound`] if the issue does not exist.
    #[allow(clippy::too_many_arguments)]
    pub fn update_issue(
        &self,
        reference: &str,
        title: Option<&str>,
        body: Option<&str>,
        state: Option<&IssueState>,
        add_labels: &[&str],
        remove_labels: &[&str],
        add_assignees: &[&str],
        remove_assignees: &[&str],
        source_url: Option<&str>,
    ) -> Result<Issue> {
        self.store().update_issue(
            reference,
            title,
            body,
            state,
            add_labels,
            remove_labels,
            add_assignees,
            remove_assignees,
            source_url,
        )
    }
    // -----------------------------------------------------------------------
    // Reviews
    // -----------------------------------------------------------------------

    /// Create a new review.
    ///
    /// # Errors
    /// Returns an error if a git operation fails.
    pub fn create_review(
        &self,
        title: &str,
        body: &str,
        target: &ReviewTarget,
        source_ref: Option<&str>,
    ) -> Result<Review> {
        self.store().create_review(title, body, target, source_ref)
    }

    /// Fetch a review by display ID or OID prefix.
    ///
    /// # Errors
    /// Returns [`crate::Error::NotFound`] if no matching review exists.
    pub fn get_review(&self, reference: &str) -> Result<Review> {
        self.store().get_review(reference)
    }

    /// List all reviews, optionally filtered by state.
    ///
    /// # Errors
    /// Returns an error if a git operation fails.
    pub fn list_reviews(&self, state: Option<&ReviewState>) -> Result<Vec<Review>> {
        match state {
            Some(s) => self.store().list_reviews_by_state(s),
            None => self.store().list_reviews(),
        }
    }

    /// Apply a partial update to a review.
    ///
    /// # Errors
    /// Returns [`crate::Error::NotFound`] if the review does not exist.
    pub fn update_review(
        &self,
        reference: &str,
        title: Option<&str>,
        body: Option<&str>,
        state: Option<&ReviewState>,
    ) -> Result<Review> {
        self.store().update_review(reference, title, body, state)
    }

    /// Approve a review as the current git user.
    ///
    /// # Errors
    /// Returns an error if the review does not exist or a git operation fails.
    pub fn approve_review(&self, reference: &str, contributor_uuid: &str) -> Result<Review> {
        self.store().approve_review(reference, contributor_uuid)
    }

    /// Revoke the current user's approval on a review.
    ///
    /// # Errors
    /// Returns an error if the review does not exist or a git operation fails.
    pub fn revoke_approval(&self, reference: &str, contributor_uuid: &str) -> Result<Review> {
        self.store().revoke_approval(reference, contributor_uuid)
    }

    /// Retarget a review to a new head, migrating carry-forward comments.
    ///
    /// Returns the updated review.
    ///
    /// # Errors
    /// Returns an error if the review does not exist or a git operation fails.
    pub fn retarget_review(&self, reference: &str, new_head: &str) -> Result<Review> {
        let resolved_head = resolve_to_oid(&self.repo, new_head)?;
        let (_, review) = self.store().retarget_review(reference, &resolved_head)?;
        Ok(review)
    }

    // -----------------------------------------------------------------------
    // Comment API
    // -----------------------------------------------------------------------

    /// Create a new comment thread, optionally anchored to a git object.
    ///
    /// When `anchor` targets a blob with a line range, context lines are
    /// extracted automatically.
    ///
    /// # Errors
    /// Returns an error if a git operation fails.
    pub fn create_comment(
        &self,
        body: &str,
        anchor: Option<&Anchor>,
        context_lines: Option<&str>,
    ) -> Result<(String, Comment)> {
        let ctx = match anchor {
            Some(a) if a.start_line.is_some() && context_lines.is_none() => {
                extract_context(&self.repo, &a.oid, a.start_line.unwrap_or(1), a.end_line).ok()
            }
            _ => context_lines.map(str::to_string),
        };
        create_thread(&self.repo, body, anchor, ctx.as_deref())
    }

    /// Append a reply to an existing comment thread.
    ///
    /// # Errors
    /// Returns an error if the thread or reply-to OID cannot be found.
    pub fn reply_comment(
        &self,
        thread_id: &str,
        body: &str,
        reply_to_oid: &str,
        anchor: Option<&Anchor>,
        context_lines: Option<&str>,
    ) -> Result<Comment> {
        let ctx = match anchor {
            Some(a) if a.start_line.is_some() && context_lines.is_none() => {
                extract_context(&self.repo, &a.oid, a.start_line.unwrap_or(1), a.end_line).ok()
            }
            _ => context_lines.map(str::to_string),
        };
        reply_to_thread(
            &self.repo,
            thread_id,
            body,
            reply_to_oid,
            anchor,
            ctx.as_deref(),
        )
    }

    /// Resolve a comment thread.
    ///
    /// # Errors
    /// Returns an error if the thread or reply-to OID cannot be found.
    pub fn resolve_comment_thread(
        &self,
        thread_id: &str,
        reply_to_oid: &str,
        message: Option<&str>,
    ) -> Result<Comment> {
        resolve_thread(&self.repo, thread_id, reply_to_oid, message)
    }

    /// List all comments anchored to a git object (blob, commit, tree, or tag).
    ///
    /// Finds all v2 threads anchored to `oid`, flattens their comments,
    /// and returns them sorted by timestamp.
    ///
    /// # Errors
    /// Returns an error if a git operation fails.
    pub fn list_comments_on(&self, oid: &str) -> Result<Vec<Comment>> {
        let thread_ids = find_threads_by_object(&self.repo, oid)?;
        let mut comments = Vec::new();
        for thread_id in &thread_ids {
            let cs = list_thread_comments(&self.repo, thread_id)?;
            comments.extend(cs);
        }
        comments.sort_by_key(|c| c.timestamp);
        Ok(comments)
    }

    /// Resolve an anchor spec string to a git object OID.
    ///
    /// Accepts:
    /// - A 40-character hex OID → returned as-is
    /// - A `HEAD:<path>` spec → resolved to blob OID at HEAD
    /// - `issue:<display-id>` → resolved to the issue's root commit OID
    /// - `review:<display-id>` → resolved to the review's root commit OID
    ///
    /// # Errors
    /// Returns an error if the spec cannot be resolved.
    pub fn resolve_anchor_spec(&self, spec: &str) -> Result<String> {
        if spec.len() == 40
            && let Ok(oid) = git2::Oid::from_str(spec)
        {
            return Ok(oid.to_string());
        }
        if let Some(path) = spec.strip_prefix("HEAD:") {
            let head_commit = self.repo.head()?.peel_to_commit()?;
            let tree = head_commit.tree()?;
            let entry = tree
                .get_path(std::path::Path::new(path))
                .map_err(|_| Error::NotFound(spec.to_string()))?;
            return Ok(entry.id().to_string());
        }
        if let Some(display_id) = spec.strip_prefix("issue:") {
            let issue = self.store().get_issue(display_id)?;
            return Ok(issue.oid);
        }
        if let Some(display_id) = spec.strip_prefix("review:") {
            let review = self.store().get_review(display_id)?;
            return Ok(review.oid);
        }
        // Fall back to git rev-parse.
        let obj = self.repo.revparse_single(spec)?;
        Ok(obj.id().to_string())
    }

    // -----------------------------------------------------------------------
    // Review worktree
    // -----------------------------------------------------------------------

    /// Return the active review OID if running inside a review worktree.
    #[must_use]
    pub fn active_review(&self) -> Option<String> {
        if !self.repo.is_worktree() {
            return None;
        }
        let marker = self.repo.path().join("forge-review");
        std::fs::read_to_string(marker)
            .ok()
            .map(|s| s.trim().to_string())
    }

    /// Print the status of the active review (unresolved comments, approvals).
    ///
    /// # Errors
    /// Returns an error if not in a review worktree or a git operation fails.
    pub fn review_status(&self, json: bool) -> Result<()> {
        let review_oid = self
            .active_review()
            .ok_or_else(|| Error::Config("not in a review worktree".into()))?;
        let review = self.store().get_review(&review_oid)?;

        if json {
            print_review(&review, true);
            return Ok(());
        }

        let label = review
            .display_id
            .as_deref()
            .unwrap_or(&review_oid[..review_oid.len().min(12)]);
        println!("Review {label}: {}", review.title);
        println!("State:  {:?}", review.state);

        // Collect unresolved comment threads grouped by file.
        let files = review_target_files(&self.repo, &review)?;
        let mut unresolved_by_file: std::collections::BTreeMap<String, usize> =
            std::collections::BTreeMap::new();
        let mut total_unresolved = 0usize;
        let mut total_resolved = 0usize;

        for (path, blob_oid) in &files {
            let Ok(thread_ids) = crate::comment::find_threads_by_object(&self.repo, blob_oid)
            else {
                continue;
            };
            for tid in &thread_ids {
                let resolved = crate::comment::thread_is_resolved(&self.repo, tid).unwrap_or(false);
                if resolved {
                    total_resolved += 1;
                } else {
                    total_unresolved += 1;
                    *unresolved_by_file.entry(path.clone()).or_default() += 1;
                }
            }
        }

        println!("Threads: {total_unresolved} unresolved, {total_resolved} resolved");

        if !unresolved_by_file.is_empty() {
            println!();
            for (path, count) in &unresolved_by_file {
                println!("  {path}  ({count})");
            }
        }

        if review.approvals.is_empty() {
            println!("\nNot approved");
        } else {
            println!();
            for entry in &review.approvals {
                println!(
                    "Approved[{}]: {}",
                    &entry.oid[..entry.oid.len().min(12)],
                    entry.approvers.join(", ")
                );
            }
        }

        Ok(())
    }

    /// Check out a review into a git worktree.
    ///
    /// Creates a worktree at `path` (or a default location) with the review's
    /// head checked out, and writes a `forge-review` marker so that subsequent
    /// commands inside the worktree can detect the active review.
    ///
    /// If the worktree already exists for this review, returns its path without
    /// recreating it.
    ///
    /// # Errors
    /// Returns an error if the review is not found, the review target is not a
    /// commit, or a git operation fails.
    pub fn checkout_review(
        &self,
        reference: &str,
        path: Option<&Path>,
    ) -> Result<(Review, std::path::PathBuf)> {
        let review = self.store().get_review(reference)?;

        let wt_name = format!("forge-review-{}", &review.oid[..12.min(review.oid.len())]);

        // If the worktree already exists and is valid, return its path.
        // If it exists but is stale, prune it so we can recreate below.
        if let Ok(wt) = self.repo.find_worktree(&wt_name) {
            if wt.validate().is_ok() {
                let wt_repo = Repository::open(wt.path())?;
                let wt_workdir = wt_repo
                    .workdir()
                    .ok_or_else(|| Error::Config("worktree has no workdir".into()))?;
                return Ok((review, wt_workdir.to_path_buf()));
            }
            let _ = wt.prune(Some(
                git2::WorktreePruneOptions::new()
                    .valid(false)
                    .working_tree(true),
            ));
        }

        // Remove any orphaned worktree admin directory left by a prior failed
        // checkout or incomplete prune so that `repo.worktree()` can succeed.
        let stale_admin = self.repo.path().join("worktrees").join(&wt_name);
        if stale_admin.exists() {
            let _ = std::fs::remove_dir_all(&stale_admin);
        }

        let workdir = self
            .repo
            .workdir()
            .ok_or_else(|| Error::Config("bare repository".into()))?;
        let repo_name = workdir
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("repo");

        let label = review
            .display_id
            .as_deref()
            .unwrap_or(&review.oid[..12.min(review.oid.len())]);
        // Sanitize label for use as a path component.
        let safe_label: String = label
            .chars()
            .map(|c| if c == '/' { '_' } else { c })
            .collect();
        let default_path = workdir
            .parent()
            .unwrap_or(workdir)
            .join(format!("{repo_name}@{safe_label}"));
        let wt_path = path.unwrap_or(&default_path);

        // Resolve the head to a commit. If the target is a tree, create a
        // synthetic orphan commit so we can attach a worktree without altering
        // the review itself.
        let head_oid = git2::Oid::from_str(&review.target.head)?;
        let head_obj = self.repo.find_object(head_oid, None)?;
        let head_commit = match head_obj.kind() {
            Some(git2::ObjectType::Commit) => head_obj.peel_to_commit()?,
            Some(git2::ObjectType::Tree) => {
                let tree = head_obj.peel_to_tree()?;
                let sig = self.repo.signature()?;
                let synthetic_oid = self.repo.commit(
                    None,
                    &sig,
                    &sig,
                    &format!(
                        "forge: synthetic commit for review {}",
                        &review.oid[..12.min(review.oid.len())]
                    ),
                    &tree,
                    &[],
                )?;
                self.repo.find_commit(synthetic_oid)?
            }
            _ => {
                return Err(Error::Config(format!(
                    "review target {} is not a commit or tree",
                    &review.target.head[..12.min(review.target.head.len())]
                )));
            }
        };

        // Create a branch for the worktree so it has a clean ref.
        let branch_name = format!("forge/review/{}", &safe_label);
        let branch = self.repo.branch(&branch_name, &head_commit, true)?;
        let branch_ref = branch.into_reference();

        if let Some(parent) = wt_path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        let mut opts = git2::WorktreeAddOptions::new();
        opts.reference(Some(&branch_ref));
        let wt = self.repo.worktree(&wt_name, wt_path, Some(&opts))?;

        // Write the marker file.
        let wt_repo = Repository::open(wt.path())?;
        let marker_path = wt_repo.path().join("forge-review");
        std::fs::write(&marker_path, &review.oid)?;

        Ok((review, wt_path.to_path_buf()))
    }

    /// Remove a review worktree created by [`checkout_review`].
    ///
    /// Prunes the worktree, removes its working directory, and deletes the
    /// `forge/review/*` branch that was created for it.
    ///
    /// If `reference` is `None`, the active review is inferred from the current
    /// worktree context.
    ///
    /// # Errors
    /// Returns an error if no review context can be determined or a git
    /// operation fails.
    pub fn done_review(&self, reference: Option<&str>) -> Result<Review> {
        let review_oid = match reference {
            Some(r) => {
                let review = self.store().get_review(r)?;
                review.oid.clone()
            }
            None => self
                .active_review()
                .ok_or_else(|| Error::Config("not in a review worktree".into()))?,
        };

        let review = self.store().get_review(&review_oid)?;
        let wt_name = format!("forge-review-{}", &review_oid[..12.min(review_oid.len())]);

        let label = review
            .display_id
            .as_deref()
            .unwrap_or(&review_oid[..12.min(review_oid.len())]);
        let safe_label: String = label
            .chars()
            .map(|c| if c == '/' { '_' } else { c })
            .collect();

        // Find and remove the worktree.
        if let Ok(wt) = self.repo.find_worktree(&wt_name) {
            // Remove the working directory first.
            if let Ok(wt_repo) = Repository::open(wt.path())
                && let Some(wd) = wt_repo.workdir()
            {
                let _ = std::fs::remove_dir_all(wd);
            }
            wt.prune(Some(
                git2::WorktreePruneOptions::new()
                    .valid(true)
                    .working_tree(true),
            ))?;
        }

        // Delete the branch we created.
        let branch_name = format!("forge/review/{safe_label}");
        if let Ok(mut branch) = self.repo.find_branch(&branch_name, git2::BranchType::Local) {
            let _ = branch.delete();
        }

        Ok(review)
    }

    // -----------------------------------------------------------------------
    // Config
    // -----------------------------------------------------------------------

    /// Auto-detect provider config from git remote URL(s).
    ///
    /// # Errors
    /// Returns an error if a remote is not found or its URL is unrecognized.
    pub fn config_init(&self, remotes: &[&str]) -> Result<Vec<ConfigEntry>> {
        let mut added = Vec::new();
        for remote_name in remotes {
            let remote = self
                .repo
                .find_remote(remote_name)
                .map_err(|_| Error::Config(format!("remote not found: {remote_name}")))?;
            let url = remote
                .url()
                .ok_or_else(|| Error::Config(format!("remote {remote_name} has no URL")))?;
            let (provider, owner, repo) = parse_remote_url(url)?;
            let sigils = default_sigils(&provider);
            let prefix = format!("provider/{provider}/{owner}/{repo}");
            let mut blobs: Vec<(String, String)> = sigils
                .iter()
                .map(|(entity, sigil)| (format!("{prefix}/sigil/{entity}"), sigil.clone()))
                .collect();
            blobs.push((format!("{prefix}/sync/issues"), "true".to_string()));
            let blob_refs: Vec<(&str, &str)> = blobs
                .iter()
                .map(|(k, v)| (k.as_str(), v.as_str()))
                .collect();
            crate::refs::write_config_blobs(&self.repo, &blob_refs)?;
            added.push(ConfigEntry {
                provider,
                owner,
                repo,
                sigils,
            });
        }
        Ok(added)
    }

    /// Manually add a provider config entry.
    ///
    /// # Errors
    /// Returns an error if a git operation fails.
    pub fn config_add(&self, provider: &str, owner: &str, repo: &str) -> Result<()> {
        let sigils = default_sigils(provider);
        let prefix = format!("provider/{provider}/{owner}/{repo}");
        let mut blobs: Vec<(String, String)> = sigils
            .iter()
            .map(|(entity, sigil)| (format!("{prefix}/sigil/{entity}"), sigil.clone()))
            .collect();
        blobs.push((format!("{prefix}/sync/issues"), "true".to_string()));
        let blob_refs: Vec<(&str, &str)> = blobs
            .iter()
            .map(|(k, v)| (k.as_str(), v.as_str()))
            .collect();
        crate::refs::write_config_blobs(&self.repo, &blob_refs)?;
        Ok(())
    }

    /// List all configured provider entries.
    ///
    /// Returns `(provider, owner, repo, sigil)` tuples.
    ///
    /// # Errors
    /// Returns an error if a git operation fails.
    pub fn config_list(&self) -> Result<Vec<ConfigEntry>> {
        let reference = match self.repo.find_reference(crate::refs::CONFIG) {
            Ok(r) => r,
            Err(e) if e.code() == ErrorCode::NotFound => return Ok(Vec::new()),
            Err(e) => return Err(e.into()),
        };
        let root_tree = reference.peel_to_commit()?.tree()?;
        let provider_entry = match root_tree.get_path(std::path::Path::new("provider")) {
            Ok(e) => e,
            Err(e) if e.code() == ErrorCode::NotFound => return Ok(Vec::new()),
            Err(e) => return Err(e.into()),
        };
        let provider_tree = self.repo.find_tree(provider_entry.id())?;

        let mut entries = Vec::new();
        for prov_entry in &provider_tree {
            if prov_entry.kind() != Some(ObjectType::Tree) {
                continue;
            }
            let Some(provider) = prov_entry.name() else {
                continue;
            };
            let prov_tree = self.repo.find_tree(prov_entry.id())?;
            for owner_entry in &prov_tree {
                if owner_entry.kind() != Some(ObjectType::Tree) {
                    continue;
                }
                let Some(owner) = owner_entry.name() else {
                    continue;
                };
                let owner_tree = self.repo.find_tree(owner_entry.id())?;
                for repo_entry in &owner_tree {
                    if repo_entry.kind() != Some(ObjectType::Tree) {
                        continue;
                    }
                    let Some(repo_name) = repo_entry.name() else {
                        continue;
                    };
                    let sigils = crate::refs::read_config_subtree(
                        &self.repo,
                        &format!("provider/{provider}/{owner}/{repo_name}/sigil"),
                    )?;
                    entries.push(ConfigEntry {
                        provider: provider.to_string(),
                        owner: owner.to_string(),
                        repo: repo_name.to_string(),
                        sigils,
                    });
                }
            }
        }
        Ok(entries)
    }

    /// Resolve a remote name to a `(provider, owner, repo)` tuple.
    fn resolve_remote(&self, remote_name: &str) -> Result<(String, String, String)> {
        let remote = self
            .repo
            .find_remote(remote_name)
            .map_err(|_| Error::Config(format!("remote not found: {remote_name}")))?;
        let url = remote
            .url()
            .ok_or_else(|| Error::Config(format!("remote {remote_name} has no URL")))?;
        parse_remote_url(url)
    }

    /// Edit sigils and sync scopes for a provider config resolved from a
    /// git remote.
    ///
    /// # Errors
    /// Returns an error if the remote cannot be parsed, the config entry does
    /// not exist, or a git operation fails.
    pub fn config_edit(
        &self,
        remote_name: &str,
        add_sigils: &[(String, String)],
        remove_sigils: &[String],
        add_sync: &[String],
        remove_sync: &[String],
    ) -> Result<()> {
        let (provider, owner, repo) = self.resolve_remote(remote_name)?;
        let prefix = format!("provider/{provider}/{owner}/{repo}");

        for entity in remove_sigils {
            crate::refs::remove_config_blob(&self.repo, &format!("{prefix}/sigil/{entity}"))?;
        }
        for scope in remove_sync {
            match scope.as_str() {
                "issues" | "reviews" => {
                    crate::refs::remove_config_blob(&self.repo, &format!("{prefix}/sync/{scope}"))?;
                }
                other => {
                    return Err(Error::Config(format!("unknown sync scope: {other}")));
                }
            }
        }

        // Validate add_sync scopes before batching.
        for scope in add_sync {
            match scope.as_str() {
                "issues" | "reviews" => {}
                other => {
                    return Err(Error::Config(format!("unknown sync scope: {other}")));
                }
            }
        }

        // Batch all writes into a single commit.
        let mut blobs: Vec<(String, String)> = add_sigils
            .iter()
            .map(|(key, value)| (format!("{prefix}/sigil/{key}"), value.clone()))
            .collect();
        for scope in add_sync {
            blobs.push((format!("{prefix}/sync/{scope}"), "true".to_string()));
        }
        if !blobs.is_empty() {
            let blob_refs: Vec<(&str, &str)> = blobs
                .iter()
                .map(|(k, v)| (k.as_str(), v.as_str()))
                .collect();
            crate::refs::write_config_blobs(&self.repo, &blob_refs)?;
        }

        Ok(())
    }

    /// Read current sigils and sync scopes for a config entry.
    fn config_read_current(
        &self,
        remote_name: &str,
    ) -> Result<(
        std::collections::BTreeMap<String, String>,
        std::collections::BTreeMap<String, String>,
    )> {
        let (provider, owner, repo) = self.resolve_remote(remote_name)?;
        let prefix = format!("provider/{provider}/{owner}/{repo}");
        let sigils = crate::refs::read_config_subtree(&self.repo, &format!("{prefix}/sigil"))?;
        let sync = crate::refs::read_config_subtree(&self.repo, &format!("{prefix}/sync"))?;
        Ok((sigils, sync))
    }

    /// Remove a provider config entry.
    ///
    /// # Errors
    /// Returns an error if the entry does not exist.
    pub fn config_remove(&self, provider: &str, owner: &str, repo: &str) -> Result<()> {
        let reference = self
            .repo
            .find_reference(crate::refs::CONFIG)
            .map_err(|_| Error::Config(format!("no config entry for {provider}/{owner}/{repo}")))?;
        let parent = reference.peel_to_commit()?;
        let root_tree = parent.tree()?;

        let owner_path = format!("provider/{provider}/{owner}");
        let owner_entry = root_tree
            .get_path(std::path::Path::new(&owner_path))
            .map_err(|_| Error::Config(format!("no config entry for {provider}/{owner}/{repo}")))?;
        let owner_tree = self.repo.find_tree(owner_entry.id())?;
        let mut owner_builder = self.repo.treebuilder(Some(&owner_tree))?;
        owner_builder
            .remove(repo)
            .map_err(|_| Error::Config(format!("no config entry for {provider}/{owner}/{repo}")))?;

        // Rebuild upward from owner → provider → root.
        let new_owner_oid = owner_builder.write()?;
        let root_oid = rebuild_tree_upward(
            &self.repo,
            &root_tree,
            &["provider", provider, owner],
            new_owner_oid,
        )?;
        let new_root = self.repo.find_tree(root_oid)?;
        let sig = self.repo.signature()?;
        self.repo.commit(
            Some(crate::refs::CONFIG),
            &sig,
            &sig,
            "forge: remove config entry",
            &new_root,
            &[&parent],
        )?;
        Ok(())
    }
}

/// Detect and launch an editor for the given path.
///
/// Detection order: `$FORGE_EDITOR`, `$VISUAL`, `$EDITOR`, then probe for
/// `zed`, `code`, `nvim`, `vim` in `$PATH`.  GUI editors (`zed`, `code`)
/// are spawned and detached; TUI editors (`nvim`, `vim`) run in the
/// foreground.
fn launch_editor(path: &std::path::Path) -> Result<()> {
    let gui_editors = ["zed", "code"];
    let tui_editors = ["nvim", "vim"];

    let editor = std::env::var("FORGE_EDITOR")
        .or_else(|_| std::env::var("VISUAL"))
        .or_else(|_| std::env::var("EDITOR"))
        .ok()
        .or_else(|| {
            gui_editors
                .iter()
                .chain(tui_editors.iter())
                .find(|name| which(name))
                .map(|s| (*s).to_string())
        });

    let Some(editor) = editor else {
        eprintln!("No editor found. Set $FORGE_EDITOR, $VISUAL, or $EDITOR.");
        return Ok(());
    };

    let name = std::path::Path::new(&editor)
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or(&editor);

    let is_gui = gui_editors.contains(&name);

    if is_gui {
        std::process::Command::new(&editor)
            .arg(path)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .map_err(|e| Error::Config(format!("failed to launch {editor}: {e}")))?;
    } else {
        let status = std::process::Command::new(&editor)
            .arg(path)
            .status()
            .map_err(|e| Error::Config(format!("failed to launch {editor}: {e}")))?;
        if !status.success() {
            std::process::exit(status.code().unwrap_or(1));
        }
    }
    Ok(())
}

fn which(name: &str) -> bool {
    std::env::var_os("PATH").is_some_and(|paths| {
        std::env::split_paths(&paths).any(|dir| is_executable(&dir.join(name)))
    })
}

fn is_executable(path: &std::path::Path) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::metadata(path).is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
    }
    #[cfg(not(unix))]
    {
        path.is_file()
    }
}

fn parse_remote_url(url: &str) -> Result<(String, String, String)> {
    // SSH: git@github.com:owner/repo.git
    if let Some(rest) = url.strip_prefix("git@") {
        let (host, path) = rest
            .split_once(':')
            .ok_or_else(|| Error::Config(format!("cannot parse remote URL: {url}")))?;
        let provider = host_to_provider(host)?;
        let (owner, repo) = parse_owner_repo(path)?;
        return Ok((provider, owner, repo));
    }

    // HTTPS: https://github.com/owner/repo.git
    if let Some(without_scheme) = url
        .strip_prefix("https://")
        .or_else(|| url.strip_prefix("http://"))
    {
        let mut parts = without_scheme.splitn(3, '/');
        let host = parts.next().unwrap();
        let owner = parts
            .next()
            .ok_or_else(|| Error::Config(format!("cannot parse remote URL: {url}")))?;
        let repo_raw = parts
            .next()
            .ok_or_else(|| Error::Config(format!("cannot parse remote URL: {url}")))?;
        let provider = host_to_provider(host)?;
        let repo = repo_raw.strip_suffix(".git").unwrap_or(repo_raw);
        return Ok((provider, owner.to_string(), repo.to_string()));
    }

    Err(Error::Config(format!("cannot parse remote URL: {url}")))
}

fn host_to_provider(host: &str) -> Result<String> {
    // Strip the TLD (last dot-separated segment) to get the meaningful host portion.
    let host_without_tld = match host.rsplit_once('.') {
        Some((prefix, _tld)) => prefix,
        None => host, // no dot at all, treat the whole thing as the prefix
    };
    if host_without_tld.is_empty() {
        return Err(Error::Config(format!(
            "unrecognized host: '{host}' (use `forge config add` for manual setup)"
        )));
    }

    if host.contains("github") {
        Ok("github".to_string())
    } else if host.contains("gitlab") {
        Err(Error::Config(format!(
            "unrecognized host: '{host}' (GitLab is not yet supported; use `forge config add` for manual setup)"
        )))
    } else if host.contains("gitea") {
        Err(Error::Config(format!(
            "unrecognized host: '{host}' (Gitea is not yet supported; use `forge config add` for manual setup)"
        )))
    } else if host.contains("tangled") {
        Err(Error::Config(format!(
            "unrecognized host: '{host}' (Tangled is not yet supported, but we're glad you're in the Atmosphere; use `forge config add` for manual setup)"
        )))
    } else {
        Err(Error::Config(format!(
            "unrecognized host: '{host}' (use `forge config add` for manual setup)"
        )))
    }
}

fn parse_owner_repo(path: &str) -> Result<(String, String)> {
    let (owner, repo_raw) = path
        .split_once('/')
        .ok_or_else(|| Error::Config(format!("cannot parse owner/repo from: {path}")))?;
    let repo = repo_raw.strip_suffix(".git").unwrap_or(repo_raw);
    Ok((owner.to_string(), repo.to_string()))
}

fn default_sigils(provider: &str) -> BTreeMap<String, String> {
    let (issue, review) = match provider {
        "github" => ("GH#", "GH#"),
        "gitlab" => ("GL#", "GL!"),
        "gitea" => ("GT#", "GT!"),
        "tangled" => ("T#", "T!"),
        _ => ("#", "!"),
    };
    BTreeMap::from([
        ("issue".to_string(), issue.to_string()),
        ("review".to_string(), review.to_string()),
    ])
}

/// Resolve the current git user's email to their contributor UUID.
///
/// # Errors
/// Returns an error if `user.email` is not configured or no matching
/// contributor is found.
fn current_contributor_uuid(repo: &git2::Repository, store: &crate::Store<'_>) -> Result<String> {
    let cfg = repo.config()?;
    let email = cfg
        .get_string("user.email")
        .map_err(|_| Error::Config("user.email not set".into()))?;
    let map = store.email_to_contributor_map()?;
    map.get(&email)
        .map(|id| id.as_str().to_string())
        .ok_or_else(|| {
            Error::Config(format!(
                "no contributor found for email {email}; run `forge contributor bootstrap` first"
            ))
        })
}

/// Rebuild a tree chain from the leaf upward, replacing the entry at the
/// given path segments with `new_oid`.
fn rebuild_tree_upward(
    repo: &Repository,
    root: &git2::Tree<'_>,
    segments: &[&str],
    new_oid: git2::Oid,
) -> Result<git2::Oid> {
    // Walk down, collecting (segment, parent_tree_oid) pairs.
    let mut pairs: Vec<(&str, git2::Oid)> = Vec::new();
    let mut current_oid = root.id();
    for &seg in segments {
        pairs.push((seg, current_oid));
        let tree = repo.find_tree(current_oid)?;
        current_oid = tree
            .get_name(seg)
            .ok_or_else(|| Error::Config(format!("missing tree entry: {seg}")))?
            .id();
    }

    // Fold bottom-up: each parent re-inserts its child.
    let mut child_oid = new_oid;
    for (seg, parent_oid) in pairs.into_iter().rev() {
        let parent = repo.find_tree(parent_oid)?;
        let mut builder = repo.treebuilder(Some(&parent))?;
        builder.insert(seg, child_oid, 0o040_000)?;
        child_oid = builder.write()?;
    }
    Ok(child_oid)
}

#[cfg(feature = "cli")]
impl Executor {
    /// Resolve a working-tree path to a git object OID.
    ///
    /// If the path is clean, resolves via `HEAD:<path>`. If dirty and
    /// `allow_dirty` is set, hashes the working-tree content into the object
    /// store and returns the resulting OID.
    #[doc(hidden)]
    pub fn resolve_path(&self, path: &std::path::Path, allow_dirty: bool) -> Result<String> {
        let workdir = self
            .repo
            .workdir()
            .ok_or_else(|| Error::Config("bare repository has no working tree".into()))?;
        let abs = workdir.join(path);
        let clean_oid = self
            .repo
            .revparse_single(&format!("HEAD:{}", path.display()))
            .ok()
            .map(|o| o.id());

        let (dirty, working_oid) = if abs.is_file() {
            let current = self.repo.blob_path(&abs)?;
            (clean_oid.is_none_or(|oid| oid != current), Some(current))
        } else if abs.is_dir() {
            let mut opts = git2::StatusOptions::new();
            opts.include_untracked(false)
                .include_ignored(false)
                .pathspec(format!("{}/*", path.display()));
            let statuses = self.repo.statuses(Some(&mut opts))?;
            (!statuses.is_empty(), None)
        } else {
            return Err(Error::NotFound(path.display().to_string()));
        };

        if !dirty {
            return Ok(clean_oid.unwrap().to_string());
        }

        if !allow_dirty {
            return Err(Error::DirtyWorktree);
        }

        // Return the already-hashed blob OID, or hash the dirty directory.
        if let Some(oid) = working_oid {
            Ok(oid.to_string())
        } else {
            Ok(hash_worktree_dir(&self.repo, &abs)?.to_string())
        }
    }

    /// Resolve a `--head` revspec to a git object OID.
    ///
    /// When `allow_dirty` is set and the revspec references HEAD (either the
    /// commit itself or a `HEAD:<path>` subtree), the working-tree content is
    /// hashed into the object store instead.
    #[doc(hidden)]
    pub fn resolve_head(&self, spec: &str, allow_dirty: bool) -> Result<String> {
        // HEAD:<path> — delegate to resolve_path for dirty-aware handling.
        if let Some(path) = spec.strip_prefix("HEAD:") {
            return self.resolve_path(std::path::Path::new(path), allow_dirty);
        }

        let obj = self
            .repo
            .revparse_single(spec)
            .map_err(|_| Error::NotFound(spec.to_string()))?;

        if !allow_dirty {
            return Ok(obj.id().to_string());
        }

        // Check if the resolved object points at HEAD (commit or its tree).
        if let Ok(head_ref) = self.repo.head()
            && let Ok(head_commit) = head_ref.peel_to_commit()
            && let Some(workdir) = self.repo.workdir()
        {
            let is_head_commit = obj.id() == head_commit.id();
            let is_head_tree = obj.id() == head_commit.tree_id();
            if is_head_commit || is_head_tree {
                return Ok(hash_worktree_dir(&self.repo, workdir)?.to_string());
            }
        }

        Ok(obj.id().to_string())
    }

    /// Resolve `--issue`, `--review`, or `--object` to the comment chain ref name.
    ///
    /// Dispatch a parsed CLI command, writing output to stdout.
    ///
    /// # Errors
    /// Returns an error if the underlying forge operation fails.
    ///
    /// # Panics
    /// Panics if facet-json fails to serialize a value (indicates a bug).
    #[allow(clippy::too_many_lines)]
    pub fn run(&self, cli: &crate::cli::Cli) -> Result<()> {
        use crate::cli::{
            Command, CommentCommand, ConfigCommand, ContributorCommand, IssueCommand, ReviewCommand,
        };
        use crate::comment::{edit_in_thread, find_thread_by_comment, list_thread_comments};
        use crate::contributor::Contributor;

        match &cli.command {
            Command::Contributor { command } => match command {
                ContributorCommand::Init {
                    handle,
                    names,
                    emails,
                    roles,
                    no_interactive,
                } => {
                    let store = self.store();
                    let existing = store.list_contributors()?;
                    let sig = self.repo.signature()?;
                    let default_name = sig.name().unwrap_or("unknown").to_string();
                    let default_email = sig.email().unwrap_or("unknown").to_string();

                    if existing.is_empty() {
                        // Bootstrap: first contributor gets admin role.
                        let interactive = !no_interactive && should_interact(handle.is_none());
                        let (eff_handle, eff_names, eff_emails) = if interactive {
                            let default_handle = default_handle_from_name(&default_name);
                            let input = crate::interactive::prompt_init_contributor(
                                &default_handle,
                                &default_name,
                                &default_email,
                            )?;
                            (input.handle, input.names, input.emails)
                        } else {
                            let h = handle
                                .clone()
                                .unwrap_or_else(|| default_handle_from_name(&default_name));
                            let n = if names.is_empty() {
                                vec![default_name.clone()]
                            } else {
                                names.clone()
                            };
                            let e = if emails.is_empty() {
                                vec![default_email.clone()]
                            } else {
                                emails.clone()
                            };
                            (h, n, e)
                        };
                        let eff_roles = if roles.is_empty() {
                            vec!["admin".to_string()]
                        } else {
                            roles.clone()
                        };
                        let names_ref: Vec<&str> = eff_names.iter().map(String::as_str).collect();
                        let emails_ref: Vec<&str> = eff_emails.iter().map(String::as_str).collect();
                        let roles_ref: Vec<&str> = eff_roles.iter().map(String::as_str).collect();
                        let c = store.create_contributor(
                            &eff_handle,
                            &names_ref,
                            &emails_ref,
                            &roles_ref,
                        )?;
                        if cli.json {
                            println!("{}", facet_json::to_string_pretty(&c).expect("serialize"));
                        } else {
                            println!("bootstrapped contributor {} ({})", c.handle, c.id);
                        }
                    } else {
                        // Check if caller is already a contributor.
                        let email_map = store.email_to_contributor_map()?;
                        if email_map.contains_key(&default_email) {
                            return Err(Error::Config(
                                "you are already a contributor; use `contributor edit` instead"
                                    .into(),
                            ));
                        }

                        // Add new contributor (no admin role by default).
                        let interactive = !no_interactive && should_interact(handle.is_none());
                        let (eff_handle, eff_names, eff_emails) = if interactive {
                            let default_handle = default_handle_from_name(&default_name);
                            let input = crate::interactive::prompt_init_contributor(
                                &default_handle,
                                &default_name,
                                &default_email,
                            )?;
                            (input.handle, input.names, input.emails)
                        } else {
                            let h = handle
                                .clone()
                                .unwrap_or_else(|| default_handle_from_name(&default_name));
                            let n = if names.is_empty() {
                                vec![default_name.clone()]
                            } else {
                                names.clone()
                            };
                            let e = if emails.is_empty() {
                                vec![default_email.clone()]
                            } else {
                                emails.clone()
                            };
                            (h, n, e)
                        };
                        let roles_ref: Vec<&str> = roles.iter().map(String::as_str).collect();
                        let names_ref: Vec<&str> = eff_names.iter().map(String::as_str).collect();
                        let emails_ref: Vec<&str> = eff_emails.iter().map(String::as_str).collect();
                        let c = store.create_contributor(
                            &eff_handle,
                            &names_ref,
                            &emails_ref,
                            &roles_ref,
                        )?;
                        if cli.json {
                            println!("{}", facet_json::to_string_pretty(&c).expect("serialize"));
                        } else {
                            println!("added contributor {} ({})", c.handle, c.id);
                        }
                    }
                }

                ContributorCommand::List => {
                    let contributors = self.store().list_contributors()?;
                    if cli.json {
                        println!(
                            "{}",
                            facet_json::to_string_pretty(&contributors).expect("serialize")
                        );
                    } else {
                        for c in &contributors {
                            let roles = if c.roles.is_empty() {
                                String::new()
                            } else {
                                format!("  roles={}", c.roles.join(","))
                            };
                            println!("{}  {}{}", c.id, c.handle, roles);
                        }
                    }
                }

                ContributorCommand::Show { reference } => {
                    let c: Contributor = if let Ok(c) = self.store().get_contributor(reference) {
                        c
                    } else {
                        let id = self.store().resolve_handle(reference)?;
                        self.store().get_contributor(id.as_str())?
                    };
                    if cli.json {
                        println!("{}", facet_json::to_string_pretty(&c).expect("serialize"));
                    } else {
                        println!("id:     {}", c.id);
                        println!("handle: {}", c.handle);
                        if !c.names.is_empty() {
                            println!("names:  {}", c.names.join(", "));
                        }
                        if !c.emails.is_empty() {
                            println!("emails: {}", c.emails.join(", "));
                        }
                        if !c.roles.is_empty() {
                            println!("roles:  {}", c.roles.join(", "));
                        }
                        if !c.keys.is_empty() {
                            println!("keys:   {}", c.keys.join(", "));
                        }
                    }
                }

                ContributorCommand::Rename { old, new } => {
                    let c = self.store().rename_contributor(old, new)?;
                    if cli.json {
                        println!("{}", facet_json::to_string_pretty(&c).expect("serialize"));
                    } else {
                        println!("renamed contributor {} → {} ({})", old, c.handle, c.id);
                    }
                }

                ContributorCommand::Edit {
                    handle,
                    add_names,
                    remove_names,
                    add_emails,
                    remove_emails,
                    add_keys,
                    key_file,
                    remove_keys,
                    add_roles,
                    remove_roles,
                    interactive,
                } => {
                    let no_flags = add_names.is_empty()
                        && remove_names.is_empty()
                        && add_emails.is_empty()
                        && remove_emails.is_empty()
                        && add_keys.is_empty()
                        && remove_keys.is_empty()
                        && add_roles.is_empty()
                        && remove_roles.is_empty();
                    let interactive = *interactive || should_interact(no_flags);

                    let (
                        eff_add_names,
                        eff_remove_names,
                        eff_add_emails,
                        eff_remove_emails,
                        eff_add_roles,
                        eff_remove_roles,
                    ) = if interactive {
                        let current_id = self.store().resolve_handle(handle)?;
                        let current = self.store().get_contributor(current_id.as_str())?;
                        let input = crate::interactive::prompt_edit_contributor(&current)?;
                        (
                            input.add_names,
                            input.remove_names,
                            input.add_emails,
                            input.remove_emails,
                            input.add_roles,
                            input.remove_roles,
                        )
                    } else {
                        (
                            add_names.clone(),
                            remove_names.clone(),
                            add_emails.clone(),
                            remove_emails.clone(),
                            add_roles.clone(),
                            remove_roles.clone(),
                        )
                    };

                    let store = self.store();
                    let mut c: Option<Contributor> = None;
                    for name in &eff_remove_names {
                        c = Some(store.remove_contributor_name(handle, name)?);
                    }
                    for name in &eff_add_names {
                        c = Some(store.add_contributor_name(handle, name)?);
                    }
                    for email in &eff_remove_emails {
                        c = Some(store.remove_contributor_email(handle, email)?);
                    }
                    for email in &eff_add_emails {
                        c = Some(store.add_contributor_email(handle, email)?);
                    }
                    for fp in remove_keys {
                        c = Some(store.remove_contributor_key(handle, fp)?);
                    }
                    for fp in add_keys {
                        let material = if let Some(path) = key_file {
                            std::fs::read(path)?
                        } else {
                            use std::io::Read;
                            let mut buf = Vec::new();
                            std::io::stdin().read_to_end(&mut buf)?;
                            buf
                        };
                        c = Some(store.add_contributor_key(handle, fp, &material)?);
                    }
                    for role in &eff_remove_roles {
                        c = Some(store.remove_contributor_role(handle, role)?);
                    }
                    for role in &eff_add_roles {
                        c = Some(store.add_contributor_role(handle, role)?);
                    }

                    if let Some(c) = c {
                        if cli.json {
                            println!("{}", facet_json::to_string_pretty(&c).expect("serialize"));
                        } else {
                            println!("updated contributor {}", c.handle);
                        }
                    } else {
                        println!("no changes");
                    }
                }
            },

            Command::Config { command } => match command {
                ConfigCommand::Init { remote } => {
                    let remote = remote.as_deref().unwrap_or("origin");
                    let added = self.config_init(&[remote])?;
                    if cli.json {
                        println!(
                            "{}",
                            facet_json::to_string_pretty(&added).expect("serialize")
                        );
                    } else {
                        for entry in &added {
                            println!("added {}/{}/{}", entry.provider, entry.owner, entry.repo);
                        }
                    }
                }

                ConfigCommand::Add {
                    provider,
                    owner,
                    repo,
                } => {
                    self.config_add(provider, owner, repo)?;
                    if !cli.json {
                        println!("added {provider}/{owner}/{repo}");
                    }
                }

                ConfigCommand::List => {
                    let entries = self.config_list()?;
                    if cli.json {
                        println!(
                            "{}",
                            facet_json::to_string_pretty(&entries).expect("serialize")
                        );
                    } else {
                        for entry in &entries {
                            let sigils: Vec<String> = entry
                                .sigils
                                .iter()
                                .map(|(k, v)| format!("{k}={v}"))
                                .collect();
                            println!(
                                "{}/{}/{}  {}",
                                entry.provider,
                                entry.owner,
                                entry.repo,
                                sigils.join(" "),
                            );
                        }
                    }
                }

                ConfigCommand::Remove {
                    provider,
                    owner,
                    repo,
                } => {
                    self.config_remove(provider, owner, repo)?;
                    if !cli.json {
                        println!("removed {provider}/{owner}/{repo}");
                    }
                }

                ConfigCommand::Edit {
                    remote,
                    add_sigils,
                    remove_sigils,
                    add_sync,
                    remove_sync,
                    interactive,
                } => {
                    let no_flags = add_sigils.is_empty()
                        && remove_sigils.is_empty()
                        && add_sync.is_empty()
                        && remove_sync.is_empty();
                    let interactive = *interactive || should_interact(no_flags);

                    let (eff_add_sigils, eff_remove_sigils, eff_add_sync, eff_remove_sync) =
                        if interactive {
                            let (current_sigils, current_sync) =
                                self.config_read_current(remote)?;
                            let input = crate::interactive::prompt_edit_config(
                                &current_sigils,
                                &current_sync,
                            )?;
                            (
                                input.add_sigils,
                                input.remove_sigils,
                                input.add_sync,
                                input.remove_sync,
                            )
                        } else {
                            (
                                add_sigils.clone(),
                                remove_sigils.clone(),
                                add_sync.clone(),
                                remove_sync.clone(),
                            )
                        };

                    self.config_edit(
                        remote,
                        &eff_add_sigils,
                        &eff_remove_sigils,
                        &eff_add_sync,
                        &eff_remove_sync,
                    )?;
                    if !cli.json {
                        println!("updated config for remote {remote}");
                    }
                }

                ConfigCommand::Reindex => {
                    let store = self.store();
                    let issues = store.reindex_issues()?;
                    let reviews = store.reindex_reviews()?;
                    eprintln!("reindexed {issues} issues, {reviews} reviews");
                }
            },

            Command::Comment { command } => match command {
                CommentCommand::Create {
                    on,
                    lines,
                    body,
                    file,
                    interactive,
                } => {
                    let resolved = crate::input::resolve_body(body.clone(), file.clone())?;
                    let interactive = *interactive || should_interact(resolved.is_none());
                    let body = if interactive {
                        crate::interactive::prompt_body(resolved.as_deref())?
                    } else {
                        resolved.unwrap_or_default()
                    };
                    let oid = self.resolve_anchor_spec(on)?;
                    let anchor = build_v2_anchor(&oid, lines.as_deref());
                    let (thread_id, comment) = self.create_comment(&body, Some(&anchor), None)?;
                    if cli.json {
                        println!(
                            "{}",
                            facet_json::to_string_pretty(&comment).expect("serialize")
                        );
                    } else {
                        println!("thread: {thread_id}");
                        print_comment(&comment, false);
                    }
                }

                CommentCommand::Reply {
                    reply_to,
                    body,
                    file,
                    interactive,
                } => {
                    let thread_id = find_thread_by_comment(&self.repo, reply_to)?
                        .ok_or_else(|| Error::NotFound(reply_to.clone()))?;
                    let resolved = crate::input::resolve_body(body.clone(), file.clone())?;
                    let interactive = *interactive || should_interact(resolved.is_none());
                    let body = if interactive {
                        crate::interactive::prompt_body(resolved.as_deref())?
                    } else {
                        resolved.unwrap_or_default()
                    };
                    let comment = self.reply_comment(&thread_id, &body, reply_to, None, None)?;
                    print_comment(&comment, cli.json);
                }

                CommentCommand::Resolve {
                    comment,
                    message,
                    file,
                    interactive,
                } => {
                    let thread_id = find_thread_by_comment(&self.repo, comment)?
                        .ok_or_else(|| Error::NotFound(comment.clone()))?;
                    let resolved = crate::input::resolve_body(message.clone(), file.clone())?;
                    let interactive = *interactive || should_interact(resolved.is_none());
                    let resolved = if interactive {
                        Some(crate::interactive::prompt_body(resolved.as_deref())?)
                    } else {
                        resolved
                    };
                    let result =
                        self.resolve_comment_thread(&thread_id, comment, resolved.as_deref())?;
                    print_comment(&result, cli.json);
                }

                CommentCommand::Edit { comment, body } => {
                    let thread_id = find_thread_by_comment(&self.repo, comment)?
                        .ok_or_else(|| Error::NotFound(comment.clone()))?;
                    let result = edit_in_thread(&self.repo, &thread_id, comment, body, None, None)?;
                    print_comment(&result, cli.json);
                }

                CommentCommand::List { on, all, state } => {
                    let comments = if *all {
                        let thread_ids = list_all_thread_ids(&self.repo)?;
                        let mut acc = Vec::new();
                        for thread_id in &thread_ids {
                            let resolved = thread_is_resolved(&self.repo, thread_id)?;
                            let include = match state {
                                CommentStateFilter::Active => !resolved,
                                CommentStateFilter::Resolved => resolved,
                                CommentStateFilter::All => true,
                            };
                            if include {
                                let cs = list_thread_comments(&self.repo, thread_id)?;
                                acc.extend(cs);
                            }
                        }
                        acc.sort_by_key(|c| c.timestamp);
                        acc
                    } else {
                        let oid = self
                            .resolve_anchor_spec(on.as_deref().expect("--on or --all required"))?;
                        self.list_comments_on(&oid)?
                    };
                    if cli.json {
                        println!(
                            "{}",
                            facet_json::to_string_pretty(&comments).expect("serialize")
                        );
                    } else {
                        print_comment_list(&comments);
                    }
                }

                CommentCommand::Show { comment } => {
                    let thread_id = find_thread_by_comment(&self.repo, comment)?
                        .ok_or_else(|| Error::NotFound(comment.clone()))?;
                    let comments = list_thread_comments(&self.repo, &thread_id)?;
                    if cli.json {
                        println!(
                            "{}",
                            facet_json::to_string_pretty(&comments).expect("serialize")
                        );
                    } else {
                        print_comment_list(&comments);
                    }
                }
            },

            Command::Review { command: None } => {
                self.review_status(cli.json)?;
            }
            Command::Review {
                command: Some(command),
            } => match command {
                ReviewCommand::New {
                    title,
                    body,
                    file,
                    head,
                    path,
                    base,
                    source_ref,
                    interactive,
                } => {
                    let resolved_body = crate::input::resolve_body(body.clone(), file.clone())?;
                    let interactive =
                        *interactive || should_interact(title.is_none() && resolved_body.is_none());
                    let (title, body) = if interactive {
                        let input = crate::interactive::prompt_new_review(title.as_deref())?;
                        (input.title, input.body)
                    } else {
                        (
                            title.clone().unwrap_or_default(),
                            resolved_body.unwrap_or_default(),
                        )
                    };
                    let title = title.as_str();
                    let body = body.as_str();

                    // Resolve target to a git object OID.
                    let head_oid = match (head, path) {
                        (Some(h), _) => self.resolve_head(h, cli.allow_dirty)?,
                        (None, Some(p)) => self.resolve_path(p, cli.allow_dirty)?,
                        _ => {
                            return Err(Error::Config("--head or --path required".into()));
                        }
                    };
                    let base_oid = base
                        .as_deref()
                        .map(|b| {
                            self.repo
                                .revparse_single(b)
                                .map(|o| o.id().to_string())
                                .map_err(|_| Error::NotFound(b.to_string()))
                        })
                        .transpose()?;
                    let target = ReviewTarget {
                        head: head_oid,
                        base: base_oid,
                    };
                    let review = self.create_review(title, body, &target, source_ref.as_deref())?;
                    print_review(&review, cli.json);
                }

                ReviewCommand::Show { reference } => {
                    let review = self.get_review(reference)?;
                    print_review(&review, cli.json);
                }

                ReviewCommand::List { state } => {
                    let all_flag = state
                        .as_deref()
                        .is_some_and(|s| s.eq_ignore_ascii_case("all"));
                    let states: Vec<ReviewState> = state
                        .as_deref()
                        .filter(|s| !s.eq_ignore_ascii_case("all"))
                        .map(|s| {
                            s.split(',')
                                .map(|v| v.trim().parse())
                                .collect::<Result<Vec<_>>>()
                        })
                        .transpose()?
                        .unwrap_or_else(|| vec![ReviewState::Open, ReviewState::Draft]);

                    let reviews = if all_flag {
                        self.list_reviews(None)?
                    } else if states.len() == 1 {
                        self.list_reviews(Some(&states[0]))?
                    } else {
                        let mut all = self.list_reviews(None)?;
                        all.retain(|r| states.contains(&r.state));
                        all
                    };

                    if cli.json {
                        println!(
                            "{}",
                            facet_json::to_string_pretty(&reviews).expect("serialize")
                        );
                    } else {
                        let color = std::io::stdout().is_terminal();
                        print_review_list(&reviews, color);
                    }
                }

                ReviewCommand::Edit {
                    reference,
                    title,
                    body,
                    file,
                    state,
                    interactive,
                } => {
                    let resolved_body = crate::input::resolve_body(body.clone(), file.clone())?;
                    let no_fields = title.is_none() && resolved_body.is_none() && state.is_none();
                    let interactive = *interactive || should_interact(no_fields);
                    let (eff_title, eff_body, eff_state): (
                        Option<String>,
                        Option<String>,
                        Option<ReviewState>,
                    ) = if interactive {
                        let current = self.get_review(reference)?;
                        let input = crate::interactive::prompt_edit_review(&current)?;
                        (
                            input.title.or_else(|| title.clone()),
                            input.body.or_else(|| resolved_body.clone()),
                            input.state.or_else(|| state.clone()),
                        )
                    } else {
                        (title.clone(), resolved_body, state.clone())
                    };
                    let review = self.update_review(
                        reference,
                        eff_title.as_deref(),
                        eff_body.as_deref(),
                        eff_state.as_ref(),
                    )?;
                    print_review(&review, cli.json);
                }

                ReviewCommand::Close { reference } => {
                    let review =
                        self.update_review(reference, None, None, Some(&ReviewState::Closed))?;
                    print_review(&review, cli.json);
                }

                ReviewCommand::Merge { reference } => {
                    let review =
                        self.update_review(reference, None, None, Some(&ReviewState::Merged))?;
                    print_review(&review, cli.json);
                }

                ReviewCommand::Approve { reference, path } => {
                    let uuid = current_contributor_uuid(&self.repo, &self.store())?;
                    let review = if let Some(p) = path {
                        let blob_oid = self.resolve_path(p, cli.allow_dirty)?;
                        self.store()
                            .approve_review_object(reference, &blob_oid, &uuid)?
                    } else {
                        self.approve_review(reference, &uuid)?
                    };
                    print_review(&review, cli.json);
                }

                ReviewCommand::Unapprove { reference } => {
                    let uuid = current_contributor_uuid(&self.repo, &self.store())?;
                    let review = self.revoke_approval(reference, &uuid)?;
                    print_review(&review, cli.json);
                }

                ReviewCommand::Files { reference } => {
                    let review = self.get_review(reference)?;
                    let files = review_target_files(&self.repo, &review)?;
                    if cli.json {
                        println!(
                            "{}",
                            facet_json::to_string_pretty(&files).expect("serialize")
                        );
                    } else if files.is_empty() {
                        println!("No files found (target object may not exist locally).");
                    } else {
                        for (path, oid) in &files {
                            println!("{} {path}", &oid[..oid.len().min(12)]);
                        }
                    }
                }

                ReviewCommand::Coverage { revision } => {
                    let (covered, uncovered) = review_coverage(&self.repo, self, revision)?;
                    let total = covered.len() + uncovered.len();
                    if cli.json {
                        let report = CoverageReport {
                            total,
                            covered: covered.len(),
                            uncovered_files: &uncovered,
                        };
                        println!(
                            "{}",
                            facet_json::to_string_pretty(&report).expect("serialize")
                        );
                    } else if total == 0 {
                        println!("No blobs found in {revision}.");
                    } else if uncovered.is_empty() {
                        println!("All {total} blobs covered by approved reviews.");
                    } else {
                        println!("{}/{total} blobs covered. Uncovered:\n", covered.len());
                        for (path, oid) in &uncovered {
                            println!("  {} {path}", &oid[..oid.len().min(12)]);
                        }
                    }
                }

                ReviewCommand::Start {
                    reference,
                    path,
                    no_editor,
                } => {
                    let (review, wt_path) = self.checkout_review(reference, path.as_deref())?;
                    if cli.json {
                        print_review(&review, true);
                    } else {
                        let label = review
                            .display_id
                            .as_deref()
                            .unwrap_or(&review.oid[..review.oid.len().min(12)]);
                        println!("Checked out review {label} to {}", wt_path.display());
                        if !no_editor {
                            launch_editor(&wt_path)?;
                        }
                    }
                }

                ReviewCommand::Finish { reference } => {
                    let review = self.done_review(reference.as_deref())?;
                    if cli.json {
                        print_review(&review, true);
                    } else {
                        let label = review
                            .display_id
                            .as_deref()
                            .unwrap_or(&review.oid[..review.oid.len().min(12)]);
                        println!("Removed review worktree for {label}");
                    }
                }

                ReviewCommand::Retarget { reference, head } => {
                    let review = self.retarget_review(reference, head)?;
                    if cli.json {
                        print_review(&review, true);
                    } else {
                        let label = review
                            .display_id
                            .as_deref()
                            .unwrap_or(&review.oid[..review.oid.len().min(12)]);
                        println!("Retargeted review {label} to {}", &head);
                    }
                }
            },

            Command::Issue { command } => match command {
                IssueCommand::New {
                    title,
                    body,
                    file,
                    labels,
                    assignees,
                    interactive,
                } => {
                    let resolved_body = crate::input::resolve_body(body.clone(), file.clone())?;
                    let interactive =
                        *interactive || should_interact(title.is_none() && resolved_body.is_none());
                    let (title, body, labels, assignees) = if interactive {
                        let input = crate::interactive::prompt_new_issue(title.as_deref())?;
                        (input.title, input.body, input.labels, input.assignees)
                    } else {
                        (
                            title.clone().unwrap_or_default(),
                            resolved_body.unwrap_or_default(),
                            labels.clone(),
                            assignees.clone(),
                        )
                    };
                    let labels_ref: Vec<&str> = labels.iter().map(String::as_str).collect();
                    let assignees_ref: Vec<&str> = assignees.iter().map(String::as_str).collect();
                    let issue = self.create_issue(&title, &body, &labels_ref, &assignees_ref)?;
                    print_issue(&issue, cli.json);
                }

                IssueCommand::Show { reference } => {
                    let issue = self.get_issue(reference)?;
                    print_issue(&issue, cli.json);
                }

                IssueCommand::List {
                    state,
                    platform,
                    id,
                } => {
                    let split = |s: &str| -> Vec<String> {
                        s.split(',')
                            .map(|v| v.trim().to_string())
                            .filter(|v| !v.is_empty())
                            .collect()
                    };

                    let states: Vec<IssueState> = state
                        .as_deref()
                        .filter(|s| !s.eq_ignore_ascii_case("all"))
                        .map(|s| {
                            split(s)
                                .iter()
                                .map(|v| v.parse())
                                .collect::<Result<Vec<_>>>()
                        })
                        .transpose()?
                        .unwrap_or_default();
                    let platforms: Vec<String> = platform
                        .as_deref()
                        .filter(|s| !s.eq_ignore_ascii_case("all"))
                        .map(&split)
                        .unwrap_or_default();
                    let ids: Vec<String> = id.as_deref().map(split).unwrap_or_default();

                    let mut issues = if states.len() == 1 {
                        self.list_issues(Some(&states[0]))?
                    } else {
                        let mut all = self.list_issues(None)?;
                        if !states.is_empty() {
                            all.retain(|i| states.contains(&i.state));
                        }
                        all
                    };

                    if !platforms.is_empty() {
                        issues.retain(|i| {
                            i.display_id.as_deref().is_some_and(|id| {
                                platforms.iter().any(|pfx| id.starts_with(pfx.as_str()))
                            })
                        });
                    }

                    if !ids.is_empty() {
                        issues.retain(|i| {
                            ids.iter().any(|needle| {
                                i.display_id
                                    .as_deref()
                                    .is_some_and(|id| id == needle.as_str())
                                    || i.oid.starts_with(needle.as_str())
                            })
                        });
                    }

                    if cli.json {
                        println!(
                            "{}",
                            facet_json::to_string_pretty(&issues).expect("serialize")
                        );
                    } else {
                        let color = std::io::stdout().is_terminal();
                        let single_state = if states.len() == 1 {
                            Some(&states[0])
                        } else {
                            None
                        };
                        print_issue_list(&issues, single_state, color);
                    }
                }

                IssueCommand::Edit {
                    reference,
                    title,
                    body,
                    file,
                    state,
                    add_labels,
                    remove_labels,
                    add_assignees,
                    remove_assignees,
                    source_url,
                    interactive,
                } => {
                    let resolved_body = crate::input::resolve_body(body.clone(), file.clone())?;
                    let no_fields = title.is_none()
                        && resolved_body.is_none()
                        && state.is_none()
                        && add_labels.is_empty()
                        && remove_labels.is_empty()
                        && add_assignees.is_empty()
                        && remove_assignees.is_empty()
                        && source_url.is_none();
                    let interactive = *interactive || should_interact(no_fields);
                    let (eff_title, eff_body, eff_state): (
                        Option<String>,
                        Option<String>,
                        Option<IssueState>,
                    ) = if interactive {
                        let current = self.get_issue(reference)?;
                        let input = crate::interactive::prompt_edit_issue(&current)?;
                        (
                            input.title.or_else(|| title.clone()),
                            input.body.or_else(|| resolved_body.clone()),
                            input.state.or_else(|| state.clone()),
                        )
                    } else {
                        (title.clone(), resolved_body, state.clone())
                    };
                    let add_labels: Vec<&str> = add_labels.iter().map(String::as_str).collect();
                    let remove_labels: Vec<&str> =
                        remove_labels.iter().map(String::as_str).collect();
                    let add_assignees: Vec<&str> =
                        add_assignees.iter().map(String::as_str).collect();
                    let remove_assignees: Vec<&str> =
                        remove_assignees.iter().map(String::as_str).collect();
                    let issue = self.update_issue(
                        reference,
                        eff_title.as_deref(),
                        eff_body.as_deref(),
                        eff_state.as_ref(),
                        &add_labels,
                        &remove_labels,
                        &add_assignees,
                        &remove_assignees,
                        source_url.as_deref(),
                    )?;
                    print_issue(&issue, cli.json);
                }

                IssueCommand::Close { reference } => {
                    let issue = self.update_issue(
                        reference,
                        None,
                        None,
                        Some(&IssueState::Closed),
                        &[],
                        &[],
                        &[],
                        &[],
                        None,
                    )?;
                    print_issue(&issue, cli.json);
                }

                IssueCommand::Reopen { reference } => {
                    let issue = self.update_issue(
                        reference,
                        None,
                        None,
                        Some(&IssueState::Open),
                        &[],
                        &[],
                        &[],
                        &[],
                        None,
                    )?;
                    print_issue(&issue, cli.json);
                }
            },

            #[cfg(feature = "server")]
            Command::Server { command } => {
                use crate::cli::ServerCommand;
                match command {
                    ServerCommand::Start {
                        poll_interval,
                        remote,
                        no_sync_refs,
                        once,
                        foreground,
                    } => {
                        self.server_start(
                            *poll_interval,
                            remote,
                            *no_sync_refs,
                            *once,
                            *foreground,
                        )?;
                    }
                    ServerCommand::Stop => {
                        self.server_stop()?;
                    }
                    ServerCommand::Status => {
                        self.server_status()?;
                    }
                }
            }
        }
        Ok(())
    }
}

#[cfg(feature = "server")]
impl Executor {
    fn pidfile_path(&self) -> std::path::PathBuf {
        self.repo.path().join("forge-server.pid")
    }

    fn read_pid(&self) -> Result<Option<u32>> {
        let path = self.pidfile_path();
        match std::fs::read_to_string(&path) {
            Ok(s) => {
                let pid: u32 = s.trim().parse().map_err(|_| {
                    crate::Error::Sync(format!("invalid pid in {}", path.display()))
                })?;
                Ok(Some(pid))
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    fn is_process_alive(pid: u32) -> bool {
        // `kill -0` checks if process exists without sending a signal.
        std::process::Command::new("kill")
            .args(["-0", &pid.to_string()])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .is_ok_and(|s| s.success())
    }

    fn server_start(
        &self,
        poll_interval: u64,
        remote: &str,
        no_sync_refs: bool,
        once: bool,
        foreground: bool,
    ) -> Result<()> {
        // Check for already-running daemon.
        if let Some(pid) = self.read_pid()? {
            if Self::is_process_alive(pid) {
                eprintln!("forge-server already running (pid {pid})");
                return Ok(());
            }
            // Stale pidfile — remove it.
            let _ = std::fs::remove_file(self.pidfile_path());
        }

        if !once && !foreground && std::io::IsTerminal::is_terminal(&std::io::stdin()) {
            let confirm = inquire::Confirm::new("Start forge sync daemon in the background?")
                .with_default(true)
                .prompt()
                .map_err(|e| crate::Error::Sync(e.to_string()))?;
            if !confirm {
                return Ok(());
            }
        }

        let repo_path = self
            .repo
            .workdir()
            .or_else(|| self.repo.path().parent())
            .unwrap_or(self.repo.path());

        let mut cmd = std::process::Command::new("forge-server");
        cmd.arg("--repo").arg(repo_path);
        cmd.arg("--poll-interval").arg(poll_interval.to_string());
        cmd.arg("--remote").arg(remote);
        if no_sync_refs {
            cmd.arg("--no-sync-refs");
        }
        if once {
            cmd.arg("--once");
        }

        if once || foreground {
            // Run in foreground — exec and wait.
            let status = cmd
                .status()
                .map_err(|e| crate::Error::Sync(format!("failed to start forge-server: {e}")))?;
            if !status.success() {
                return Err(crate::Error::Sync(format!(
                    "forge-server exited with {status}"
                )));
            }
        } else {
            // Spawn detached background process.
            cmd.stdin(std::process::Stdio::null());
            cmd.stdout(std::process::Stdio::null());
            cmd.stderr(std::process::Stdio::null());

            let child = cmd
                .spawn()
                .map_err(|e| crate::Error::Sync(format!("failed to start forge-server: {e}")))?;

            let pid = child.id();
            std::fs::write(self.pidfile_path(), format!("{pid}\n"))
                .map_err(|e| crate::Error::Sync(format!("failed to write pidfile: {e}")))?;
            eprintln!("forge-server started (pid {pid})");
        }

        Ok(())
    }

    fn server_stop(&self) -> Result<()> {
        let Some(pid) = self.read_pid()? else {
            eprintln!("forge-server is not running");
            return Ok(());
        };

        if !Self::is_process_alive(pid) {
            let _ = std::fs::remove_file(self.pidfile_path());
            eprintln!("forge-server is not running (stale pidfile removed)");
            return Ok(());
        }

        let _ = std::process::Command::new("kill")
            .args([&pid.to_string()])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status();

        // Wait briefly for the process to exit and clean up its pidfile.
        for _ in 0..10 {
            if !Self::is_process_alive(pid) {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(100));
        }

        // Remove pidfile if the process didn't clean it up itself.
        let _ = std::fs::remove_file(self.pidfile_path());

        if Self::is_process_alive(pid) {
            eprintln!("forge-server (pid {pid}) did not exit, sending SIGKILL");
            let _ = std::process::Command::new("kill")
                .args(["-9", &pid.to_string()])
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .status();
        } else {
            eprintln!("forge-server stopped (pid {pid})");
        }

        Ok(())
    }

    fn server_status(&self) -> Result<()> {
        let Some(pid) = self.read_pid()? else {
            eprintln!("forge-server is not running");
            return Ok(());
        };

        if Self::is_process_alive(pid) {
            eprintln!("forge-server is running (pid {pid})");
        } else {
            let _ = std::fs::remove_file(self.pidfile_path());
            eprintln!("forge-server is not running (stale pidfile removed)");
        }
        Ok(())
    }
}

fn print_comment(comment: &Comment, json: bool) {
    if json {
        println!(
            "{}",
            facet_json::to_string_pretty(comment).expect("serialize")
        );
        return;
    }
    println!("comment {}", &comment.oid[..comment.oid.len().min(8)]);
    println!(
        "author:  {} <{}>",
        comment.author_name, comment.author_email
    );
    if comment.resolved {
        println!("resolved");
    }
    if let Some(ref r) = comment.replaces {
        println!("replaces: {}", &r[..8.min(r.len())]);
    }
    if !comment.body.is_empty() {
        println!();
        println!("{}", render_markdown(&comment.body));
    }
}

fn print_comment_list(comments: &[Comment]) {
    if comments.is_empty() {
        println!("No comments.");
        return;
    }
    println!("{} comment(s)\n", comments.len());
    for c in comments {
        let short = &c.oid[..c.oid.len().min(8)];
        let reply_marker = if c.reply_to.is_some() { "↳ " } else { "" };
        let resolved_marker = if c.resolved { " [resolved]" } else { "" };
        println!("{reply_marker}{short}{resolved_marker}  {}", c.author_name);
        if !c.body.is_empty() {
            let preview: String = c
                .body
                .lines()
                .next()
                .unwrap_or("")
                .chars()
                .take(72)
                .collect();
            println!("  {preview}");
        }
        println!();
    }
}

#[cfg(feature = "cli")]
fn render_markdown(text: &str) -> String {
    format!("{}", termimad::inline(text))
}

#[cfg(not(feature = "cli"))]
fn render_markdown(text: &str) -> String {
    text.to_string()
}

fn print_issue(issue: &Issue, json: bool) {
    if json {
        println!(
            "{}",
            facet_json::to_string_pretty(issue).expect("serialize")
        );
        return;
    }
    let id = issue.display_id.as_deref().unwrap_or(&issue.oid);
    let prefix = if id.starts_with('#') { "" } else { "#" };
    println!("issue {prefix}{id}");
    println!("title:     {}", issue.title);
    println!("state:     {}", issue.state.as_str());
    if !issue.labels.is_empty() {
        println!("labels:    {}", issue.labels.join(", "));
    }
    if !issue.assignees.is_empty() {
        println!("assignees: {}", issue.assignees.join(", "));
    }
    if !issue.body.is_empty() {
        println!();
        println!("{}", render_markdown(&issue.body));
    }
}

/// Extract `(sigil, number)` from a display ID like `"GH#4"` → `("GH#", 4)`.
fn parse_display_id(id: &str) -> (&str, u64) {
    let num_start = id.find(|c: char| c.is_ascii_digit()).unwrap_or(id.len());
    let (prefix, num_str) = id.split_at(num_start);
    let num = num_str.parse().unwrap_or(u64::MAX);
    (prefix, num)
}

fn print_issue_list(issues: &[Issue], filter: Option<&IssueState>, color: bool) {
    use comfy_table::{Cell, Table};

    if issues.is_empty() {
        println!("No issues found.");
        return;
    }

    let mut sorted: Vec<&Issue> = issues.iter().collect();
    sorted.sort_by(|a, b| {
        let (sa, na) = parse_display_id(a.display_id.as_deref().unwrap_or(""));
        let (sb, nb) = parse_display_id(b.display_id.as_deref().unwrap_or(""));
        sa.cmp(sb).then(na.cmp(&nb))
    });

    // Summary line.
    match filter {
        Some(s) => println!("Showing {} {} issues\n", sorted.len(), s.as_str()),
        None => println!("Showing {} issues\n", sorted.len()),
    }

    // Determine zero-pad width from the largest number.
    let max_num: u64 = sorted
        .iter()
        .filter_map(|i| i.display_id.as_deref())
        .map(|id| parse_display_id(id).1)
        .max()
        .unwrap_or(0);
    let pad = max_num.max(1).ilog10() as usize + 1;

    let mut table = Table::new();
    table.load_preset(comfy_table::presets::NOTHING);

    for issue in sorted {
        let (id_str, labels_str) = if color {
            let state_color = match issue.state {
                IssueState::Open => "\x1b[32m",   // green
                IssueState::Closed => "\x1b[35m", // magenta
            };
            let reset = "\x1b[0m";
            let dim = "\x1b[2m";
            let bold = "\x1b[1m";

            let id = if let Some(id) = issue.display_id.as_deref() {
                let (prefix, num) = parse_display_id(id);
                format!("{dim}{prefix}{reset}{state_color}{bold}{num:0>pad$}{reset}")
            } else {
                format!("{dim}{}{reset}", &issue.oid[..8])
            };

            let labels = issue
                .labels
                .iter()
                .map(|l| format!("{dim}{l}{reset}"))
                .collect::<Vec<_>>()
                .join(", ");

            (id, labels)
        } else {
            let id = if let Some(id) = issue.display_id.as_deref() {
                let (prefix, num) = parse_display_id(id);
                format!("{prefix}{num:0>pad$}")
            } else {
                issue.oid[..8].to_string()
            };

            let labels = issue.labels.join(", ");
            (id, labels)
        };

        table.add_row(vec![
            Cell::new(&id_str),
            Cell::new(&issue.title),
            Cell::new(labels_str),
        ]);
    }
    println!("{table}");
}

fn print_review(review: &Review, json: bool) {
    if json {
        println!(
            "{}",
            facet_json::to_string_pretty(review).expect("serialize")
        );
        return;
    }
    let id = review.display_id.as_deref().unwrap_or(&review.oid);
    let prefix = if id.starts_with('#') { "" } else { "#" };
    println!("review {prefix}{id}");
    println!("title:       {}", review.title);
    println!("state:       {}", review.state.as_str());
    println!(
        "target.head: {}",
        &review.target.head[..review.target.head.len().min(12)]
    );
    if let Some(ref base) = review.target.base {
        println!("target.base: {}", &base[..base.len().min(12)]);
    }
    if let Some(ref sref) = review.source_ref {
        println!("ref:         {sref}");
    }
    if !review.approvals.is_empty() {
        for entry in &review.approvals {
            println!(
                "approved[{}]: {}",
                &entry.oid[..entry.oid.len().min(12)],
                entry.approvers.join(", ")
            );
        }
    }
    if !review.body.is_empty() {
        println!();
        println!("{}", render_markdown(&review.body));
    }
}

/// Resolve a ref-or-oid string to a 40-char hex OID.
fn resolve_to_oid(repo: &Repository, spec: &str) -> Result<String> {
    if let Ok(oid) = git2::Oid::from_str(spec)
        && spec.len() == 40
    {
        return Ok(oid.to_string());
    }
    let obj = repo.revparse_single(spec)?;
    Ok(obj.id().to_string())
}

/// Extract ±3 surrounding lines from a blob for a given 1-based line range.
///
/// Returns an empty string for binary blobs or OIDs that don't exist.
fn extract_context(
    repo: &Repository,
    blob_oid: &str,
    start_line: u32,
    end_line: Option<u32>,
) -> Result<String> {
    let oid = git2::Oid::from_str(blob_oid)?;
    let blob = repo.find_blob(oid)?;
    let content = std::str::from_utf8(blob.content()).unwrap_or("");
    let lines: Vec<&str> = content.lines().collect();
    let total = lines.len();

    let start = start_line.saturating_sub(1) as usize; // 0-based
    let end = end_line.unwrap_or(start_line).saturating_sub(1) as usize;
    let ctx_start = start.saturating_sub(3);
    let ctx_end = (end + 4).min(total);

    Ok(lines[ctx_start..ctx_end].join("\n"))
}

fn review_target_files(repo: &git2::Repository, review: &Review) -> Result<Vec<(String, String)>> {
    let Ok(oid) = git2::Oid::from_str(&review.target.head) else {
        return Ok(Vec::new());
    };
    let Ok(obj) = repo.find_object(oid, None) else {
        return Ok(Vec::new());
    };

    let mut files = Vec::new();
    match obj.kind() {
        Some(git2::ObjectType::Blob) => {
            files.push(("(blob)".to_string(), review.target.head.clone()));
        }
        Some(git2::ObjectType::Tree) => {
            let tree = repo.find_tree(oid)?;
            walk_tree(repo, &tree, "", &mut files);
        }
        Some(git2::ObjectType::Commit) => {
            let commit = repo.find_commit(oid)?;
            let tree = commit.tree()?;
            walk_tree(repo, &tree, "", &mut files);
        }
        _ => {}
    }
    Ok(files)
}

/// Recursively hash a working-tree directory into the object store as a tree.
///
/// Respects `.gitignore` rules, follows symlinks, and preserves the executable
/// bit on files.
#[doc(hidden)]
pub fn hash_worktree_dir(repo: &Repository, dir: &std::path::Path) -> Result<git2::Oid> {
    let mut builder = repo.treebuilder(None)?;
    let mut entries: Vec<_> = std::fs::read_dir(dir)?.collect::<std::result::Result<_, _>>()?;
    entries.sort_by_key(std::fs::DirEntry::file_name);
    for entry in entries {
        let name = entry.file_name();
        let name = name.to_string_lossy();
        let path = entry.path();

        // Skip .git directory/file (submodules use a .git file).
        if name == ".git" {
            continue;
        }

        // Respect .gitignore via libgit2.
        if repo.status_should_ignore(&path).unwrap_or(false) {
            continue;
        }

        // Follow symlinks for the canonical file type.
        let meta = std::fs::metadata(&path)?;
        if meta.is_file() {
            let oid = repo.blob_path(&path)?;
            #[cfg(unix)]
            let mode = {
                use std::os::unix::fs::PermissionsExt;
                if meta.permissions().mode() & 0o111 != 0 {
                    0o100_755
                } else {
                    0o100_644
                }
            };
            #[cfg(not(unix))]
            let mode = 0o100_644;
            builder.insert(&*name, oid, mode)?;
        } else if meta.is_dir() {
            let oid = hash_worktree_dir(repo, &path)?;
            builder.insert(&*name, oid, 0o040_000)?;
        }
    }
    Ok(builder.write()?)
}

/// Derive a default handle from a git `user.name` (lowercase first token,
/// stripped of non-alphanumeric chars).
fn default_handle_from_name(name: &str) -> String {
    let handle = name
        .split_whitespace()
        .next()
        .unwrap_or(name)
        .to_ascii_lowercase();
    let handle = handle.replace(|c: char| !c.is_alphanumeric() && c != '-' && c != '_', "");
    if handle.is_empty() {
        "contributor".to_string()
    } else {
        handle
    }
}

/// Return `true` when interactive prompts should be shown.
///
/// Requires both stdin and stdout to be a TTY *and* the `FORGE_NO_INTERACTIVE`
/// env var to be unset. `missing_input` indicates whether the caller still
/// needs user-supplied content (e.g. no `--body` was given).
#[cfg(feature = "cli")]
#[doc(hidden)]
#[must_use]
pub fn should_interact(missing_input: bool) -> bool {
    use std::io::IsTerminal;
    missing_input
        && std::io::stdin().is_terminal()
        && std::io::stdout().is_terminal()
        && std::env::var_os("FORGE_NO_INTERACTIVE").is_none()
}

/// Build a v2 `Anchor` from a resolved OID and optional `"start[-end]"` lines string.
fn build_v2_anchor(oid: &str, lines: Option<&str>) -> Anchor {
    let (start_line, end_line) = if let Some(r) = lines {
        if let Some((a, b)) = r.split_once('-') {
            (a.parse().ok(), b.parse().ok())
        } else {
            let n: Option<u32> = r.parse().ok();
            (n, n)
        }
    } else {
        (None, None)
    };
    Anchor {
        oid: oid.to_string(),
        start_line,
        end_line,
    }
}

#[derive(Serialize, Facet)]
struct CoverageReport<'a> {
    total: usize,
    covered: usize,
    uncovered_files: &'a [(String, String)],
}

type FileList = Vec<(String, String)>;

/// Compute review coverage for a revision.
///
/// Returns `(covered, uncovered)` where each is a list of `(path, blob_oid)`.
fn review_coverage(
    repo: &git2::Repository,
    executor: &Executor,
    revision: &str,
) -> Result<(FileList, FileList)> {
    // Resolve the target revision to a tree.
    let obj = repo
        .revparse_single(revision)
        .map_err(|_| Error::NotFound(revision.to_string()))?;
    let tree = match obj.kind() {
        Some(git2::ObjectType::Commit) => {
            let commit = repo.find_commit(obj.id())?;
            commit.tree()?
        }
        Some(git2::ObjectType::Tree) => repo.find_tree(obj.id())?,
        _ => {
            return Err(Error::Config(
                "revision must resolve to a commit or tree".into(),
            ));
        }
    };

    let mut all_blobs: Vec<(String, String)> = Vec::new();
    walk_tree(repo, &tree, "", &mut all_blobs);

    // Collect blob OIDs covered by any approved review.
    let approved_oids = executor.store().approved_oids()?;

    let mut covered = Vec::new();
    let mut uncovered = Vec::new();
    for (path, oid) in all_blobs {
        if approved_oids.contains(&oid) {
            covered.push((path, oid));
        } else {
            uncovered.push((path, oid));
        }
    }
    Ok((covered, uncovered))
}

fn print_review_list(reviews: &[Review], color: bool) {
    use comfy_table::{Cell, Table};

    if reviews.is_empty() {
        println!("No reviews found.");
        return;
    }

    let mut sorted: Vec<&Review> = reviews.iter().collect();
    sorted.sort_by(|a, b| {
        let (sa, na) = parse_display_id(a.display_id.as_deref().unwrap_or(""));
        let (sb, nb) = parse_display_id(b.display_id.as_deref().unwrap_or(""));
        sa.cmp(sb).then(na.cmp(&nb))
    });

    println!("Showing {} reviews\n", sorted.len());

    let max_num: u64 = sorted
        .iter()
        .filter_map(|r| r.display_id.as_deref())
        .map(|id| parse_display_id(id).1)
        .max()
        .unwrap_or(0);
    let pad = max_num.max(1).ilog10() as usize + 1;

    let mut table = Table::new();
    table.load_preset(comfy_table::presets::NOTHING);

    for review in sorted {
        let (id_str, state_str) = if color {
            let state_color = match review.state {
                ReviewState::Open => "\x1b[32m",
                ReviewState::Draft => "\x1b[33m",
                ReviewState::Closed => "\x1b[35m",
                ReviewState::Merged => "\x1b[36m",
            };
            let reset = "\x1b[0m";
            let dim = "\x1b[2m";
            let bold = "\x1b[1m";

            let id = if let Some(id) = review.display_id.as_deref() {
                let (prefix, num) = parse_display_id(id);
                format!("{dim}{prefix}{reset}{state_color}{bold}{num:0>pad$}{reset}")
            } else {
                format!("{dim}{}{reset}", &review.oid[..8])
            };
            let state = format!("{state_color}{}{reset}", review.state.as_str());
            (id, state)
        } else {
            let id = if let Some(id) = review.display_id.as_deref() {
                let (prefix, num) = parse_display_id(id);
                format!("{prefix}{num:0>pad$}")
            } else {
                review.oid[..8].to_string()
            };
            (id, review.state.as_str().to_string())
        };

        table.add_row(vec![
            Cell::new(&id_str),
            Cell::new(state_str),
            Cell::new(&review.title),
        ]);
    }
    println!("{table}");
}
