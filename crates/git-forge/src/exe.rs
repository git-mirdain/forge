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

use crate::issue::{Issue, IssueState};
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

/// Extract `(sigil, number)` from a display ID like `"GH#4"` → `("GH#", 4)`.
fn parse_display_id(id: &str) -> (&str, u64) {
    let num_start = id.find(|c: char| c.is_ascii_digit()).unwrap_or(id.len());
    let (prefix, num_str) = id.split_at(num_start);
    let num = num_str.parse().unwrap_or(u64::MAX);
    (prefix, num)
}

fn print_issue_list(issues: &[Issue]) {
    use comfy_table::{Cell, Table};

    let mut sorted: Vec<&Issue> = issues.iter().collect();
    sorted.sort_by(|a, b| {
        let (sa, na) = parse_display_id(a.display_id.as_deref().unwrap_or(""));
        let (sb, nb) = parse_display_id(b.display_id.as_deref().unwrap_or(""));
        sa.cmp(sb).then(na.cmp(&nb))
    });

    // Determine zero-pad width per sigil prefix.
    let max_num: u64 = sorted
        .iter()
        .filter_map(|i| i.display_id.as_deref())
        .map(|id| parse_display_id(id).1)
        .max()
        .unwrap_or(0);
    let pad = max_num.max(1).ilog10() as usize + 1;

    let mut table = Table::new();
    table.load_preset(comfy_table::presets::NOTHING);
    table.set_header(vec![
        Cell::new("ID"),
        Cell::new("Title"),
        Cell::new("State"),
    ]);
    for issue in sorted {
        let id_str = if let Some(id) = issue.display_id.as_deref() {
            let (prefix, num) = parse_display_id(id);
            format!("{prefix}{num:0>pad$}")
        } else {
            issue.oid[..8].to_string()
        };
        table.add_row(vec![
            Cell::new(format!("#{id_str}")),
            Cell::new(&issue.title),
            Cell::new(issue.state.as_str()),
        ]);
    }
    println!("{table}");
}
