//! Executor for forge commands.
//!
//! [`Executor`] owns a [`git2::Repository`] and exposes typed methods for each
//! forge operation. The `run` method (available with the `cli` feature) dispatches
//! from a parsed [`crate::cli::Cli`] and writes output to stdout.

use std::io::IsTerminal;
use std::path::Path;

use git2::{ErrorCode, ObjectType, Repository};

use crate::issue::{Issue, IssueState};
use crate::{Error, Result, Store};

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
    /// Auto-detect provider config from git remote URL(s).
    ///
    /// # Errors
    /// Returns an error if a remote is not found or its URL is unrecognized.
    pub fn config_init(&self, remotes: &[&str]) -> Result<Vec<(String, String, String)>> {
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
            let sigil = default_sigil(&provider);
            crate::refs::write_config_blob(
                &self.repo,
                &format!("provider/{provider}/{owner}/{repo}/sigil"),
                sigil,
            )?;
            added.push((provider, owner, repo));
        }
        Ok(added)
    }

    /// Manually add a provider config entry.
    ///
    /// # Errors
    /// Returns an error if a git operation fails.
    pub fn config_add(
        &self,
        provider: &str,
        owner: &str,
        repo: &str,
        sigil: Option<&str>,
    ) -> Result<()> {
        let sigil = sigil.unwrap_or_else(|| default_sigil(provider));
        crate::refs::write_config_blob(
            &self.repo,
            &format!("provider/{provider}/{owner}/{repo}/sigil"),
            sigil,
        )
    }

    /// List all configured provider entries.
    ///
    /// Returns `(provider, owner, repo, sigil)` tuples.
    ///
    /// # Errors
    /// Returns an error if a git operation fails.
    pub fn config_list(&self) -> Result<Vec<(String, String, String, String)>> {
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
                    let sigil = crate::refs::read_config_blob(
                        &self.repo,
                        &format!("provider/{provider}/{owner}/{repo_name}/sigil"),
                    )?
                    .unwrap_or_default();
                    entries.push((
                        provider.to_string(),
                        owner.to_string(),
                        repo_name.to_string(),
                        sigil,
                    ));
                }
            }
        }
        Ok(entries)
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
    match host {
        "github.com" => Ok("github".to_string()),
        other => Err(Error::Config(format!(
            "unrecognized host: {other} (use `forge config add` for manual setup)"
        ))),
    }
}

fn parse_owner_repo(path: &str) -> Result<(String, String)> {
    let (owner, repo_raw) = path
        .split_once('/')
        .ok_or_else(|| Error::Config(format!("cannot parse owner/repo from: {path}")))?;
    let repo = repo_raw.strip_suffix(".git").unwrap_or(repo_raw);
    Ok((owner.to_string(), repo.to_string()))
}

fn default_sigil(provider: &str) -> &str {
    match provider {
        "github" => "GH#",
        _ => "#",
    }
}

/// Rebuild a tree chain from the leaf upward, replacing the entry at the
/// given path segments with `new_oid`.
fn rebuild_tree_upward(
    repo: &Repository,
    root: &git2::Tree<'_>,
    segments: &[&str],
    new_oid: git2::Oid,
) -> Result<git2::Oid> {
    if segments.is_empty() {
        return Ok(new_oid);
    }

    // Walk down to collect trees for each segment except the last.
    let mut trees = vec![root.clone()];
    for &seg in &segments[..segments.len() - 1] {
        let oid = trees.last().unwrap().get_name(seg).unwrap().id();
        trees.push(repo.find_tree(oid)?);
    }

    // Rebuild bottom-up.
    let mut child_oid = new_oid;
    for (i, seg) in segments.iter().enumerate().rev() {
        let base = &trees[i];
        let mut builder = repo.treebuilder(Some(base))?;
        builder.insert(seg, child_oid, 0o040_000)?;
        child_oid = builder.write()?;
    }
    Ok(child_oid)
}

#[cfg(feature = "cli")]
impl Executor {
    /// Dispatch a parsed CLI command, writing output to stdout.
    ///
    /// # Errors
    /// Returns an error if the underlying forge operation fails.
    ///
    /// # Panics
    /// Panics if facet-json fails to serialize a value (indicates a bug).
    #[allow(clippy::too_many_lines)]
    pub fn run(&self, cli: &crate::cli::Cli) -> Result<()> {
        use crate::cli::{Command, ConfigCommand, IssueCommand};

        match &cli.command {
            Command::Config { command } => match command {
                ConfigCommand::Init { remote } => {
                    let remote = remote.as_deref().unwrap_or("origin");
                    let added = self.config_init(&[remote])?;
                    if cli.json {
                        let json: Vec<String> = added
                            .iter()
                            .map(|(p, o, r)| {
                                format!(r#"{{"provider":"{p}","owner":"{o}","repo":"{r}"}}"#)
                            })
                            .collect();
                        println!("[{}]", json.join(","));
                    } else {
                        for (provider, owner, repo) in &added {
                            println!("added {provider}/{owner}/{repo}");
                        }
                    }
                }

                ConfigCommand::Add {
                    provider,
                    owner,
                    repo,
                    sigil,
                } => {
                    self.config_add(provider, owner, repo, sigil.as_deref())?;
                    if !cli.json {
                        println!("added {provider}/{owner}/{repo}");
                    }
                }

                ConfigCommand::List => {
                    let entries = self.config_list()?;
                    if cli.json {
                        let json: Vec<String> = entries
                            .iter()
                            .map(|(p, o, r, s)| {
                                format!(
                                    r#"{{"provider":"{p}","owner":"{o}","repo":"{r}","sigil":"{s}"}}"#
                                )
                            })
                            .collect();
                        println!("[{}]", json.join(","));
                    } else {
                        for (provider, owner, repo, sigil) in &entries {
                            println!("{provider}/{owner}/{repo}  sigil={sigil}");
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
            },

            Command::Issue { command } => match command {
                IssueCommand::New {
                    title,
                    body,
                    labels,
                    assignees,
                    interactive,
                } => {
                    let interactive =
                        *interactive || (title.is_none() && std::io::stdin().is_terminal());
                    let (title, body, labels, assignees) = if interactive {
                        let input = crate::interactive::prompt_new_issue(title.as_deref())?;
                        (input.title, input.body, input.labels, input.assignees)
                    } else {
                        (
                            title.clone().unwrap_or_default(),
                            body.clone().unwrap_or_default(),
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

                IssueCommand::List { state } => {
                    let issues = self.list_issues(state.as_ref())?;
                    if cli.json {
                        println!(
                            "{}",
                            facet_json::to_string_pretty(&issues).expect("serialize")
                        );
                    } else {
                        print_issue_list(&issues);
                    }
                }

                IssueCommand::Edit {
                    reference,
                    title,
                    body,
                    state,
                    add_labels,
                    remove_labels,
                    add_assignees,
                    remove_assignees,
                    interactive,
                } => {
                    let no_fields = title.is_none()
                        && body.is_none()
                        && state.is_none()
                        && add_labels.is_empty()
                        && remove_labels.is_empty()
                        && add_assignees.is_empty()
                        && remove_assignees.is_empty();
                    let interactive = *interactive || (no_fields && std::io::stdin().is_terminal());
                    let (eff_title, eff_body, eff_state): (
                        Option<String>,
                        Option<String>,
                        Option<IssueState>,
                    ) = if interactive {
                        let current = self.get_issue(reference)?;
                        let input = crate::interactive::prompt_edit_issue(&current)?;
                        (
                            input.title.or_else(|| title.clone()),
                            input.body.or_else(|| body.clone()),
                            input.state.or_else(|| state.clone()),
                        )
                    } else {
                        (title.clone(), body.clone(), state.clone())
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

fn print_issue_list(issues: &[Issue]) {
    for issue in issues {
        let id = issue.display_id.as_deref().unwrap_or(&issue.oid);
        println!("#{id:<10}  {}  [{}]", issue.title, issue.state.as_str());
    }
}
