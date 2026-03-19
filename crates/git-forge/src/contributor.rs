//! Execution logic for `git forge contributor`.

use std::process;

use git2::Repository;
use git_forge::cli::ContributorSubcommand;
use git_forge_core::contributor::Contributors;

const FORGE_REFSPEC: &str = "+refs/forge/*:refs/forge/*";

fn derive_id(name: &str) -> String {
    name.split_whitespace()
        .next()
        .unwrap_or("contributor")
        .to_lowercase()
        .chars()
        .filter(|c| c.is_alphanumeric() || *c == '-')
        .collect()
}

fn fetch_forge_refs(repo: &Repository) -> Result<(), Box<dyn std::error::Error>> {
    let mut remote = repo.find_remote("origin")?;
    let mut fetch_opts = git_forge_core::credentials::fetch_options()?;
    remote.fetch(&[FORGE_REFSPEC], Some(&mut fetch_opts), None)?;
    Ok(())
}

fn push_forge_ref(
    repo: &Repository,
    ref_name: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut remote = repo.find_remote("origin")?;
    let refspec = format!("{ref_name}:{ref_name}");
    let mut push_opts = git_forge_core::credentials::push_options()?;
    remote.push(&[&refspec], Some(&mut push_opts))?;
    Ok(())
}

fn run_inner(
    command: ContributorSubcommand,
    push: bool,
    fetch: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let repo = Repository::open_from_env()?;

    match command {
        ContributorSubcommand::Add { id, name, emails } => {
            let cfg = repo.config()?;
            let name = name
                .or_else(|| cfg.get_string("user.name").ok())
                .ok_or("no name provided; set user.name in git config or pass --name")?;
            let emails = if emails.is_empty() {
                cfg.get_string("user.email")
                    .ok()
                    .map(|e| vec![e])
                    .ok_or("no email provided; set user.email in git config or pass --email")?
            } else {
                emails
            };
            let id = id.unwrap_or_else(|| derive_id(&name));

            if fetch {
                fetch_forge_refs(&repo)?;
            }
            repo.add_contributor(&id, &name, &emails)?;
            if push {
                push_forge_ref(
                    &repo,
                    git_forge_core::contributor::CONTRIBUTORS_REF,
                )?;
            }
            eprintln!("Added contributor {id} ({name} <{}>).", emails.join(", "));
        }

        ContributorSubcommand::Edit { id, name, add_emails, remove_emails } => {
            if name.is_none() && add_emails.is_empty() && remove_emails.is_empty() {
                return Err("nothing to update; pass --name, --add-email, or --remove-email".into());
            }
            if fetch {
                fetch_forge_refs(&repo)?;
            }
            repo.update_contributor(&id, name.as_deref(), &add_emails, &remove_emails)?;
            if push {
                push_forge_ref(&repo, git_forge_core::contributor::CONTRIBUTORS_REF)?;
            }
            eprintln!("Updated contributor {id}.");
        }

        ContributorSubcommand::List => {
            let contributors = repo.list_contributors()?;
            if contributors.is_empty() {
                println!("No contributors registered.");
            } else {
                for c in &contributors {
                    println!("{}\t{} <{}>", c.id, c.name, c.emails.join(", "));
                }
            }
        }

        ContributorSubcommand::Remove { id } => {
            if fetch {
                fetch_forge_refs(&repo)?;
            }
            repo.remove_contributor(&id)?;
            if push {
                push_forge_ref(&repo, git_forge_core::contributor::CONTRIBUTORS_REF)?;
            }
            eprintln!("Removed contributor {id}.");
        }

        ContributorSubcommand::Show { id } => {
            match repo.find_contributor(&id)? {
                None => {
                    eprintln!("Contributor '{id}' not found.");
                    process::exit(1);
                }
                Some(c) => {
                    println!("ID:     {}", c.id);
                    println!("Name:   {}", c.name);
                    for email in &c.emails {
                        println!("Email:  {email}");
                    }
                }
            }
        }
    }

    Ok(())
}

/// Execute a `contributor` subcommand.
pub fn run(command: ContributorSubcommand, push: bool, fetch: bool) {
    if let Err(e) = run_inner(command, push, fetch) {
        eprintln!("Error: {e}");
        process::exit(1);
    }
}
