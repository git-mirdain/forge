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

use crate::comment::{
    Anchor, Comment, add_comment, add_reply, issue_comment_ref, list_comments, object_comment_ref,
    resolve_comment, review_comment_ref,
};
use crate::issue::{Issue, IssueState};
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

/// A contributor identity stored in the forge config.
#[derive(Facet)]
pub struct Contributor {
    /// The contributor's unique identifier (typically the git user name).
    pub id: String,
    /// Known email addresses.
    pub emails: Vec<String>,
    /// Known display names.
    pub names: Vec<String>,
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
        )
    }
    /// Add a comment to an issue.
    ///
    /// # Errors
    /// Returns an error if the issue is not found or a git operation fails.
    pub fn add_issue_comment(
        &self,
        issue_ref: &str,
        body: &str,
        anchor: Option<&Anchor>,
    ) -> Result<Comment> {
        let issue = self.store().get_issue(issue_ref)?;
        let ref_name = issue_comment_ref(&issue.oid);
        add_comment(&self.repo, &ref_name, body, anchor)
    }

    /// Reply to a comment on an issue.
    ///
    /// # Errors
    /// Returns an error if the issue is not found or a git operation fails.
    pub fn reply_issue_comment(
        &self,
        issue_ref: &str,
        body: &str,
        reply_to_oid: &str,
        anchor: Option<&Anchor>,
    ) -> Result<Comment> {
        let issue = self.store().get_issue(issue_ref)?;
        let ref_name = issue_comment_ref(&issue.oid);
        add_reply(&self.repo, &ref_name, body, reply_to_oid, anchor)
    }

    /// Resolve a comment thread on an issue.
    ///
    /// # Errors
    /// Returns an error if the issue is not found or a git operation fails.
    pub fn resolve_issue_comment(
        &self,
        issue_ref: &str,
        thread_oid: &str,
        message: Option<&str>,
    ) -> Result<Comment> {
        let issue = self.store().get_issue(issue_ref)?;
        let ref_name = issue_comment_ref(&issue.oid);
        resolve_comment(&self.repo, &ref_name, thread_oid, message)
    }

    /// List comments on an issue.
    ///
    /// # Errors
    /// Returns an error if the issue is not found or a git operation fails.
    pub fn list_issue_comments(&self, issue_ref: &str) -> Result<Vec<Comment>> {
        let issue = self.store().get_issue(issue_ref)?;
        let ref_name = issue_comment_ref(&issue.oid);
        list_comments(&self.repo, &ref_name)
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
        description: &str,
        target: &ReviewTarget,
        source_ref: Option<&str>,
    ) -> Result<Review> {
        self.store()
            .create_review(title, description, target, source_ref)
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
        description: Option<&str>,
        state: Option<&ReviewState>,
    ) -> Result<Review> {
        self.store()
            .update_review(reference, title, description, state)
    }

    /// Approve a review as the current git user.
    ///
    /// # Errors
    /// Returns an error if the review does not exist or a git operation fails.
    pub fn approve_review(&self, reference: &str, message: Option<&str>) -> Result<Review> {
        self.ensure_contributor()?;
        self.store().approve_review(reference, message)
    }

    /// Revoke the current user's approval on a review.
    ///
    /// # Errors
    /// Returns an error if the review does not exist or a git operation fails.
    pub fn revoke_approval(&self, reference: &str) -> Result<Review> {
        self.store().revoke_approval(reference)
    }

    // -----------------------------------------------------------------------
    // Review comments
    // -----------------------------------------------------------------------

    /// Add a comment to a review.
    ///
    /// # Errors
    /// Returns an error if the review is not found or a git operation fails.
    pub fn add_review_comment(
        &self,
        review_ref: &str,
        body: &str,
        anchor: Option<&Anchor>,
    ) -> Result<Comment> {
        let review = self.store().get_review(review_ref)?;
        let ref_name = review_comment_ref(&review.oid);
        add_comment(&self.repo, &ref_name, body, anchor)
    }

    /// Reply to a comment on a review.
    ///
    /// # Errors
    /// Returns an error if the review is not found or a git operation fails.
    pub fn reply_review_comment(
        &self,
        review_ref: &str,
        body: &str,
        reply_to_oid: &str,
        anchor: Option<&Anchor>,
    ) -> Result<Comment> {
        let review = self.store().get_review(review_ref)?;
        let ref_name = review_comment_ref(&review.oid);
        add_reply(&self.repo, &ref_name, body, reply_to_oid, anchor)
    }

    /// Resolve a comment thread on a review.
    ///
    /// # Errors
    /// Returns an error if the review is not found or a git operation fails.
    pub fn resolve_review_comment(
        &self,
        review_ref: &str,
        thread_oid: &str,
        message: Option<&str>,
    ) -> Result<Comment> {
        let review = self.store().get_review(review_ref)?;
        let ref_name = review_comment_ref(&review.oid);
        resolve_comment(&self.repo, &ref_name, thread_oid, message)
    }

    /// List comments on a review.
    ///
    /// # Errors
    /// Returns an error if the review is not found or a git operation fails.
    pub fn list_review_comments(&self, review_ref: &str) -> Result<Vec<Comment>> {
        let review = self.store().get_review(review_ref)?;
        let ref_name = review_comment_ref(&review.oid);
        list_comments(&self.repo, &ref_name)
    }

    // -----------------------------------------------------------------------
    // Standalone object comments
    // -----------------------------------------------------------------------

    /// Add a comment on a standalone git object.
    ///
    /// # Errors
    /// Returns an error if the object is not found or a git operation fails.
    pub fn add_object_comment(
        &self,
        object_oid: &str,
        body: &str,
        anchor: Option<&Anchor>,
    ) -> Result<Comment> {
        let obj = self
            .repo
            .find_object(git2::Oid::from_str(object_oid)?, None)
            .map_err(|_| Error::NotFound(object_oid.to_string()))?;
        match obj.kind() {
            Some(ObjectType::Commit | ObjectType::Blob | ObjectType::Tag) => {}
            Some(other) => return Err(Error::InvalidObjectType(other.to_string())),
            None => return Err(Error::InvalidObjectType("unknown".into())),
        }
        let ref_name = object_comment_ref(object_oid);
        add_comment(&self.repo, &ref_name, body, anchor)
    }

    /// Reply to a comment on a standalone git object.
    ///
    /// # Errors
    /// Returns an error if a git operation fails.
    pub fn reply_object_comment(
        &self,
        object_oid: &str,
        body: &str,
        reply_to_oid: &str,
        anchor: Option<&Anchor>,
    ) -> Result<Comment> {
        let ref_name = object_comment_ref(object_oid);
        add_reply(&self.repo, &ref_name, body, reply_to_oid, anchor)
    }

    /// Resolve a comment thread on a standalone git object.
    ///
    /// # Errors
    /// Returns an error if a git operation fails.
    pub fn resolve_object_comment(
        &self,
        object_oid: &str,
        thread_oid: &str,
        message: Option<&str>,
    ) -> Result<Comment> {
        let ref_name = object_comment_ref(object_oid);
        resolve_comment(&self.repo, &ref_name, thread_oid, message)
    }

    /// List comments on a standalone git object.
    ///
    /// # Errors
    /// Returns an error if a git operation fails.
    pub fn list_object_comments(&self, object_oid: &str) -> Result<Vec<Comment>> {
        let ref_name = object_comment_ref(object_oid);
        list_comments(&self.repo, &ref_name)
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
            .join(format!("{repo_name}.review"))
            .join(&safe_label);
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
            for (entity, sigil) in &sigils {
                crate::refs::write_config_blob(
                    &self.repo,
                    &format!("{prefix}/sigil/{entity}"),
                    sigil,
                )?;
            }
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
        for (entity, sigil) in &sigils {
            crate::refs::write_config_blob(&self.repo, &format!("{prefix}/sigil/{entity}"), sigil)?;
        }
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

    /// Ensure the current git user is registered as a contributor.
    ///
    /// If a contributor entry already contains the current email, this is a
    /// no-op. Otherwise the user's name and email are appended to their
    /// contributor record (creating it if necessary).
    ///
    /// # Errors
    /// Returns an error if `user.name` or `user.email` is not configured.
    fn ensure_contributor(&self) -> Result<()> {
        let cfg = self.repo.config()?;
        let name = cfg
            .get_string("user.name")
            .map_err(|_| Error::Config("user.name not set".into()))?;
        let email = cfg
            .get_string("user.email")
            .map_err(|_| Error::Config("user.email not set".into()))?;

        let emails =
            crate::refs::read_config_subtree(&self.repo, &format!("contributors/{name}/emails"))?;
        if emails.contains_key(&email) {
            return Ok(());
        }

        self.contributor_add(&name, &[email.as_str()], &[name.as_str()])
    }

    /// Register a contributor with explicit emails and names.
    ///
    /// Entries are additive — calling this again with the same ID appends
    /// new emails and names without removing existing ones.
    ///
    /// # Errors
    /// Returns an error if a git operation fails.
    pub fn contributor_add(&self, id: &str, emails: &[&str], names: &[&str]) -> Result<()> {
        for email in emails {
            crate::refs::write_config_blob(
                &self.repo,
                &format!("contributors/{id}/emails/{email}"),
                "",
            )?;
        }
        for name in names {
            crate::refs::write_config_blob(
                &self.repo,
                &format!("contributors/{id}/names/{name}"),
                "",
            )?;
        }
        Ok(())
    }

    /// List all registered contributors.
    ///
    /// # Errors
    /// Returns an error if a git operation fails.
    pub fn contributor_list(&self) -> Result<Vec<Contributor>> {
        let reference = match self.repo.find_reference(crate::refs::CONFIG) {
            Ok(r) => r,
            Err(e) if e.code() == ErrorCode::NotFound => return Ok(Vec::new()),
            Err(e) => return Err(e.into()),
        };
        let root_tree = reference.peel_to_commit()?.tree()?;
        let contrib_entry = match root_tree.get_path(std::path::Path::new("contributors")) {
            Ok(e) => e,
            Err(e) if e.code() == ErrorCode::NotFound => return Ok(Vec::new()),
            Err(e) => return Err(e.into()),
        };
        let contrib_tree = self.repo.find_tree(contrib_entry.id())?;

        let mut contributors = Vec::new();
        for entry in &contrib_tree {
            if entry.kind() != Some(ObjectType::Tree) {
                continue;
            }
            let Some(id) = entry.name() else { continue };
            let emails =
                crate::refs::read_config_subtree(&self.repo, &format!("contributors/{id}/emails"))?;
            let names =
                crate::refs::read_config_subtree(&self.repo, &format!("contributors/{id}/names"))?;
            contributors.push(Contributor {
                id: id.to_string(),
                emails: emails.into_keys().collect(),
                names: names.into_keys().collect(),
            });
        }
        Ok(contributors)
    }

    /// Remove a contributor by ID.
    ///
    /// # Errors
    /// Returns an error if the contributor does not exist.
    pub fn contributor_remove(&self, id: &str) -> Result<()> {
        let reference = self
            .repo
            .find_reference(crate::refs::CONFIG)
            .map_err(|_| Error::Config(format!("no contributor: {id}")))?;
        let parent = reference.peel_to_commit()?;
        let root_tree = parent.tree()?;

        let contrib_entry = root_tree
            .get_path(std::path::Path::new("contributors"))
            .map_err(|_| Error::Config(format!("no contributor: {id}")))?;
        let contrib_tree = self.repo.find_tree(contrib_entry.id())?;
        let mut builder = self.repo.treebuilder(Some(&contrib_tree))?;
        builder
            .remove(id)
            .map_err(|_| Error::Config(format!("no contributor: {id}")))?;

        let new_contrib_oid = builder.write()?;
        let root_oid =
            rebuild_tree_upward(&self.repo, &root_tree, &["contributors"], new_contrib_oid)?;
        let new_root = self.repo.find_tree(root_oid)?;
        let sig = self.repo.signature()?;
        self.repo.commit(
            Some(crate::refs::CONFIG),
            &sig,
            &sig,
            "forge: remove contributor",
            &new_root,
            &[&parent],
        )?;
        Ok(())
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
    /// Validates that `--object` targets a commit, blob, or tag (not a bare tree).
    #[doc(hidden)]
    pub fn resolve_comment_entity(
        &self,
        issue: Option<&str>,
        review: Option<&str>,
        object: Option<&str>,
    ) -> Result<String> {
        if let Some(o) = object {
            let obj = self
                .repo
                .revparse_single(o)
                .map_err(|_| Error::NotFound(o.to_string()))?;
            match obj.kind() {
                Some(ObjectType::Commit | ObjectType::Blob | ObjectType::Tag) => {}
                Some(other) => return Err(Error::InvalidObjectType(other.to_string())),
                None => return Err(Error::InvalidObjectType("unknown".into())),
            }
            return Ok(object_comment_ref(&obj.id().to_string()));
        }
        let review = review.map(String::from).or_else(|| {
            if issue.is_none() {
                self.active_review()
            } else {
                None
            }
        });
        if let Some(ref r) = review {
            let review = self.store().get_review(r)?;
            return Ok(review_comment_ref(&review.oid));
        }
        let issue_ref =
            issue.ok_or_else(|| Error::Config("--issue, --review, or --object required".into()))?;
        let issue = self.store().get_issue(issue_ref)?;
        Ok(issue_comment_ref(&issue.oid))
    }

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

        match &cli.command {
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

                ConfigCommand::Contributor { command } => match command {
                    ContributorCommand::Add { id, emails, names } => {
                        let sig = self.repo.signature()?;
                        let sig_name = sig.name().unwrap_or("unknown");
                        let sig_email = sig.email().unwrap_or("unknown");

                        let id = id.as_deref().unwrap_or(sig_name);
                        let default_emails;
                        let emails: Vec<&str> = if emails.is_empty() {
                            default_emails = [sig_email.to_string()];
                            default_emails.iter().map(String::as_str).collect()
                        } else {
                            emails.iter().map(String::as_str).collect()
                        };
                        let default_names;
                        let names: Vec<&str> = if names.is_empty() {
                            default_names = [sig_name.to_string()];
                            default_names.iter().map(String::as_str).collect()
                        } else {
                            names.iter().map(String::as_str).collect()
                        };

                        self.contributor_add(id, &emails, &names)?;
                        if !cli.json {
                            println!("added contributor {id}");
                        }
                    }

                    ContributorCommand::List => {
                        let contributors = self.contributor_list()?;
                        if cli.json {
                            println!(
                                "{}",
                                facet_json::to_string_pretty(&contributors).expect("serialize")
                            );
                        } else {
                            for c in &contributors {
                                println!(
                                    "{}  emails={} names={}",
                                    c.id,
                                    c.emails.join(","),
                                    c.names.join(","),
                                );
                            }
                        }
                    }

                    ContributorCommand::Remove { id } => {
                        self.contributor_remove(id)?;
                        if !cli.json {
                            println!("removed contributor {id}");
                        }
                    }
                },
            },

            Command::Comment { command } => match command {
                CommentCommand::Add {
                    issue,
                    review,
                    object,
                    anchor,
                    anchor_path,
                    range,
                    anchor_start,
                    anchor_end,
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
                    let anchor = build_anchor(
                        anchor.as_deref(),
                        anchor_path.as_deref(),
                        range.as_deref(),
                        anchor_start.as_deref(),
                        anchor_end.as_deref(),
                    );
                    let ref_name = self.resolve_comment_entity(
                        issue.as_deref(),
                        review.as_deref(),
                        object.as_deref(),
                    )?;
                    let comment = add_comment(&self.repo, &ref_name, &body, anchor.as_ref())?;
                    print_comment(&comment, cli.json);
                }

                CommentCommand::Reply {
                    issue,
                    review,
                    object,
                    reply_to,
                    anchor,
                    anchor_path,
                    range,
                    anchor_start,
                    anchor_end,
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
                    let anchor = build_anchor(
                        anchor.as_deref(),
                        anchor_path.as_deref(),
                        range.as_deref(),
                        anchor_start.as_deref(),
                        anchor_end.as_deref(),
                    );
                    let ref_name = self.resolve_comment_entity(
                        issue.as_deref(),
                        review.as_deref(),
                        object.as_deref(),
                    )?;
                    let comment =
                        add_reply(&self.repo, &ref_name, &body, reply_to, anchor.as_ref())?;
                    print_comment(&comment, cli.json);
                }

                CommentCommand::Resolve {
                    issue,
                    review,
                    object,
                    thread,
                    message,
                    file,
                    interactive,
                } => {
                    let resolved = crate::input::resolve_body(message.clone(), file.clone())?;
                    let interactive = *interactive || should_interact(resolved.is_none());
                    let resolved = if interactive {
                        Some(crate::interactive::prompt_body(resolved.as_deref())?)
                    } else {
                        resolved
                    };
                    let ref_name = self.resolve_comment_entity(
                        issue.as_deref(),
                        review.as_deref(),
                        object.as_deref(),
                    )?;
                    let comment =
                        resolve_comment(&self.repo, &ref_name, thread, resolved.as_deref())?;
                    print_comment(&comment, cli.json);
                }

                CommentCommand::List {
                    issue,
                    review,
                    object,
                    path,
                } => {
                    let ref_name = self.resolve_comment_entity(
                        issue.as_deref(),
                        review.as_deref(),
                        object.as_deref(),
                    )?;
                    let mut comments = list_comments(&self.repo, &ref_name)?;
                    if let Some(filter_path) = path {
                        comments.retain(|c| {
                            c.anchor.as_ref().is_some_and(|a| match a {
                                Anchor::Object { path, .. } => {
                                    path.as_deref() == Some(filter_path.as_str())
                                }
                                Anchor::CommitRange { .. } => false,
                            })
                        });
                    }
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

            Command::Review { command } => match command {
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
                    let (title, description) = if interactive {
                        let input = crate::interactive::prompt_new_review(title.as_deref())?;
                        (input.title, input.description)
                    } else {
                        (
                            title.clone().unwrap_or_default(),
                            resolved_body.unwrap_or_default(),
                        )
                    };
                    let title = title.as_str();
                    let description = description.as_str();

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
                    let review =
                        self.create_review(title, description, &target, source_ref.as_deref())?;
                    print_review(&review, cli.json);
                }

                ReviewCommand::Show { reference } => {
                    let review = self.get_review(reference)?;
                    print_review(&review, cli.json);
                }

                ReviewCommand::List { state } => {
                    let states: Vec<ReviewState> = state
                        .as_deref()
                        .filter(|s| !s.eq_ignore_ascii_case("all"))
                        .map(|s| {
                            s.split(',')
                                .map(|v| v.trim().parse())
                                .collect::<Result<Vec<_>>>()
                        })
                        .transpose()?
                        .unwrap_or_default();

                    let reviews = if states.len() == 1 {
                        self.list_reviews(Some(&states[0]))?
                    } else {
                        let mut all = self.list_reviews(None)?;
                        if !states.is_empty() {
                            all.retain(|r| states.contains(&r.state));
                        }
                        all
                    };

                    if cli.json {
                        println!(
                            "{}",
                            facet_json::to_string_pretty(&reviews).expect("serialize")
                        );
                    } else {
                        print_review_list(&reviews);
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
                            input.description.or_else(|| resolved_body.clone()),
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

                ReviewCommand::Approve { reference, message } => {
                    let review = self.approve_review(reference, message.as_deref())?;
                    print_review(&review, cli.json);
                }

                ReviewCommand::Unapprove { reference } => {
                    let review = self.revoke_approval(reference)?;
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

                ReviewCommand::Checkout { reference, path } => {
                    let (review, wt_path) = self.checkout_review(reference, path.as_deref())?;
                    if cli.json {
                        print_review(&review, true);
                    } else {
                        let label = review
                            .display_id
                            .as_deref()
                            .unwrap_or(&review.oid[..review.oid.len().min(12)]);
                        println!("Checked out review {label} to {}", wt_path.display());
                        if std::io::stdin().is_terminal() {
                            let shell = std::env::var("SHELL").unwrap_or_else(|_| "sh".into());
                            let status = std::process::Command::new(&shell)
                                .current_dir(&wt_path)
                                .status()?;
                            if !status.success() {
                                std::process::exit(status.code().unwrap_or(1));
                            }
                        }
                    }
                }

                ReviewCommand::Done { reference } => {
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
                    interactive,
                } => {
                    let resolved_body = crate::input::resolve_body(body.clone(), file.clone())?;
                    let no_fields = title.is_none()
                        && resolved_body.is_none()
                        && state.is_none()
                        && add_labels.is_empty()
                        && remove_labels.is_empty()
                        && add_assignees.is_empty()
                        && remove_assignees.is_empty();
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
                    )?;
                    print_issue(&issue, cli.json);
                }
            },
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
        println!("{}", comment.body);
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

fn print_issue(issue: &Issue, json: bool) {
    if json {
        println!(
            "{}",
            facet_json::to_string_pretty(issue).expect("serialize")
        );
        return;
    }
    let id = issue.display_id.as_deref().unwrap_or(&issue.oid);
    println!("issue #{id}");
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
        println!("{}", issue.body);
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
    println!("review #{id}");
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
        let names: Vec<&str> = review.approvals.iter().map(|(n, _)| n.as_str()).collect();
        println!("approved-by: {}", names.join(", "));
    }
    if !review.description.is_empty() {
        println!();
        println!("{}", review.description);
    }
}

/// Walk the review's target object and return `(path, blob_oid)` pairs.
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

fn walk_tree(
    repo: &git2::Repository,
    tree: &git2::Tree<'_>,
    prefix: &str,
    out: &mut Vec<(String, String)>,
) {
    for entry in tree {
        let name = entry.name().unwrap_or("");
        let path = if prefix.is_empty() {
            name.to_string()
        } else {
            format!("{prefix}/{name}")
        };
        match entry.kind() {
            Some(git2::ObjectType::Blob) => {
                out.push((path, entry.id().to_string()));
            }
            Some(git2::ObjectType::Tree) => {
                if let Ok(subtree) = repo.find_tree(entry.id()) {
                    walk_tree(repo, &subtree, &path, out);
                }
            }
            _ => {}
        }
    }
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

fn build_anchor(
    anchor: Option<&str>,
    anchor_path: Option<&str>,
    range: Option<&str>,
    anchor_start: Option<&str>,
    anchor_end: Option<&str>,
) -> Option<Anchor> {
    if let Some(oid) = anchor {
        Some(Anchor::Object {
            oid: oid.to_string(),
            path: anchor_path.map(String::from),
            range: range.map(String::from),
        })
    } else if let (Some(start), Some(end)) = (anchor_start, anchor_end) {
        Some(Anchor::CommitRange {
            start: start.to_string(),
            end: end.to_string(),
        })
    } else {
        None
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
    let reviews = executor.list_reviews(None)?;
    let mut approved_oids: std::collections::HashSet<String> = std::collections::HashSet::new();
    for review in &reviews {
        if review.approvals.is_empty() {
            continue;
        }
        let files = review_target_files(repo, review)?;
        for (_, oid) in files {
            approved_oids.insert(oid);
        }
    }

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

fn print_review_list(reviews: &[Review]) {
    if reviews.is_empty() {
        println!("No reviews found.");
        return;
    }
    println!("Showing {} reviews\n", reviews.len());
    for review in reviews {
        let id = review
            .display_id
            .as_deref()
            .unwrap_or(&review.oid[..review.oid.len().min(12)]);
        println!("{id}  {}  {}", review.state.as_str(), review.title);
    }
}
