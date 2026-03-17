//! Implementation of `git forge sync`.

const FORGE_FETCH_REFSPEC: &str = "+refs/forge/*:refs/forge/*";
const FORGE_PUSH_REFSPEC: &str = "refs/forge/*:refs/forge/*";

pub fn run(
    remote: Option<&str>,
    fetch: bool,
    push: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let repo = git2::Repository::open_from_env()?;
    let remote_name = remote.unwrap_or("origin");
    let mut r = repo.find_remote(remote_name)?;

    if fetch {
        r.fetch(&[FORGE_FETCH_REFSPEC], None, None)?;
    }

    if push {
        // git2 push does not support wildcard refspecs; enumerate refs explicitly.
        let refspecs: Vec<String> = repo
            .references_glob("refs/forge/*")?
            .filter_map(|r| r.ok())
            .filter_map(|r| r.name().map(|n| format!("{n}:{n}")))
            .collect();
        if !refspecs.is_empty() {
            let refspec_strs: Vec<&str> = refspecs.iter().map(String::as_str).collect();
            let mut push_opts = git_forge_core::credentials::push_options()?;
            r.push(&refspec_strs, Some(&mut push_opts))?;
        }
    }

    Ok(())
}
