//! Implementation of `git forge install`.

const FORGE_FETCH_REFSPEC: &str = "+refs/forge/*:refs/forge/*";
const FORGE_PUSH_REFSPEC: &str = "refs/forge/*:refs/forge/*";

pub fn run(remote: Option<&str>, global: bool) -> Result<(), Box<dyn std::error::Error>> {
    let remote_name = resolve_remote(remote, global)?;
    let fetch_key = format!("remote.{remote_name}.fetch");
    let push_key = format!("remote.{remote_name}.push");

    if global {
        add_global_value(&fetch_key, FORGE_FETCH_REFSPEC)?;
        add_global_value(&push_key, FORGE_PUSH_REFSPEC)?;
        eprintln!(
            "Added forge refspecs for remote `{remote_name}` to global git config (~/.gitconfig)."
        );
    } else {
        let repo = git2::Repository::open_from_env()?;
        let mut config = repo.config()?.open_level(git2::ConfigLevel::Local)?;
        add_value_if_missing(&mut config, &fetch_key, FORGE_FETCH_REFSPEC)?;
        add_value_if_missing(&mut config, &push_key, FORGE_PUSH_REFSPEC)?;
        eprintln!("Added forge refspecs for remote `{remote_name}` to local git config.");
    }

    Ok(())
}

/// Resolve the remote name: use the provided name, or default to `origin` if it exists
/// in the local repo. Errors if no remote is given and `origin` does not exist, or if
/// `--global` is used without an explicit remote (since there is no repo to check).
fn resolve_remote(
    remote: Option<&str>,
    global: bool,
) -> Result<String, Box<dyn std::error::Error>> {
    if let Some(name) = remote {
        return Ok(name.to_string());
    }

    if global {
        return Err(
            "a remote name is required with --global (no repository to infer one from)".into(),
        );
    }

    let repo = git2::Repository::open_from_env()?;
    if repo.find_remote("origin").is_ok() {
        Ok("origin".to_string())
    } else {
        Err(
            "no remote name given and `origin` does not exist; pass a remote name explicitly"
                .into(),
        )
    }
}

/// Add `value` under `key` in the given config level, skipping if already present.
fn add_value_if_missing(
    config: &mut git2::Config,
    key: &str,
    value: &str,
) -> Result<(), git2::Error> {
    // `multivar` lets us check all values for a multi-valued key.
    let already_set = config
        .multivar(key, Some(&regex_escape(value)))
        .map(|mut entries| entries.next().is_some())
        .unwrap_or(false);

    if !already_set {
        config.set_multivar(key, "^$", value)?;
    }

    Ok(())
}

/// Add `value` under `key` in the global config, skipping if already present.
fn add_global_value(key: &str, value: &str) -> Result<(), Box<dyn std::error::Error>> {
    let global_config = git2::Config::open_default()?;
    let mut global_level = global_config.open_level(git2::ConfigLevel::Global)?;
    add_value_if_missing(&mut global_level, key, value)?;
    Ok(())
}

/// Escape a literal string for use as a regex in `Config::multivar`.
fn regex_escape(s: &str) -> String {
    s.chars()
        .flat_map(|c| {
            if "^$.*+?()[]{}|\\".contains(c) {
                vec!['\\', c]
            } else {
                vec![c]
            }
        })
        .collect()
}
