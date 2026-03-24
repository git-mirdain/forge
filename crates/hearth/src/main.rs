//! `hearth` binary — CLI entrypoint.

use std::path::PathBuf;

use clap::Parser;
use hearth::{
    Error,
    cli::{Cli, Command, ImportCommand},
    env::{
        ToolchainDef, ToolchainsConfig, load_config, load_toolchains, resolve_env, resolve_extras,
        save_toolchains,
    },
    exe::{self, Isolation},
    import::{import_dir, import_oci, import_tarball},
    store::Store,
};

fn main() {
    if let Err(e) = run() {
        eprintln!("error: {e}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), Error> {
    let cli = Cli::parse();

    let store = match cli.store {
        Some(ref p) => Store::open_or_init(&PathBuf::from(p))?,
        None => Store::open_default()?,
    };

    match cli.command {
        Command::Import(sub) => match sub {
            ImportCommand::Dir { path } => {
                let oid = import_dir(&store, &PathBuf::from(&path))?;
                println!("{oid}");
            }
            ImportCommand::Tarball { path, strip_prefix } => {
                let oid = import_tarball(&store, &PathBuf::from(&path), strip_prefix)?;
                println!("{oid}");
            }
            ImportCommand::Oci { image_ref } => {
                let oid = import_oci(&store, &image_ref)?;
                println!("{oid}");
            }
        },

        Command::Enter {
            env,
            isolation,
            config,
            toolchains,
        } => {
            let cfg = load_config(&PathBuf::from(&config))?;
            let tc = std::path::Path::new(&toolchains)
                .exists()
                .then(|| load_toolchains(&PathBuf::from(&toolchains)))
                .transpose()?;
            let env = env.as_deref().unwrap_or_else(|| cfg.default_env());
            let extras = resolve_extras(&cfg, env)?;
            let oid = resolve_env(&store, &cfg, tc.as_ref(), env)?;
            let level = Isolation::from_u8(isolation)?;
            let status = exe::enter(&store, oid, level, &extras)?;
            std::process::exit(status.code().unwrap_or(1));
        }

        Command::Run {
            env,
            isolation,
            config,
            toolchains,
            cmd,
        } => {
            let cfg = load_config(&PathBuf::from(&config))?;
            let tc = std::path::Path::new(&toolchains)
                .exists()
                .then(|| load_toolchains(&PathBuf::from(&toolchains)))
                .transpose()?;
            let env = env.as_deref().unwrap_or_else(|| cfg.default_env());
            let extras = resolve_extras(&cfg, env)?;
            let oid = resolve_env(&store, &cfg, tc.as_ref(), env)?;
            let level = Isolation::from_u8(isolation)?;
            let status = exe::run(&store, oid, level, &extras, &cmd)?;
            std::process::exit(status.code().unwrap_or(1));
        }

        Command::Hash {
            env,
            config,
            toolchains,
        } => {
            let cfg = load_config(&PathBuf::from(&config))?;
            let tc = std::path::Path::new(&toolchains)
                .exists()
                .then(|| load_toolchains(&PathBuf::from(&toolchains)))
                .transpose()?;
            let env = env.as_deref().unwrap_or_else(|| cfg.default_env());
            let oid = resolve_env(&store, &cfg, tc.as_ref(), env)?;
            println!("{oid}");
        }

        Command::Diff {
            env_a,
            env_b,
            config,
            toolchains,
        } => {
            let cfg = load_config(&PathBuf::from(&config))?;
            let tc = std::path::Path::new(&toolchains)
                .exists()
                .then(|| load_toolchains(&PathBuf::from(&toolchains)))
                .transpose()?;
            let oid_a = resolve_oid(&store, &cfg, tc.as_ref(), &env_a)?;
            let oid_b = resolve_oid(&store, &cfg, tc.as_ref(), &env_b)?;
            print_diff(store.repo(), oid_a, oid_b)?;
        }

        Command::Checkout {
            env,
            path,
            direnv,
            config,
            toolchains,
        } => {
            let cfg = load_config(&PathBuf::from(&config))?;
            let tc = std::path::Path::new(&toolchains)
                .exists()
                .then(|| load_toolchains(&PathBuf::from(&toolchains)))
                .transpose()?;
            let env = env.as_deref().unwrap_or_else(|| cfg.default_env());
            let extras = resolve_extras(&cfg, env)?;
            let oid = resolve_env(&store, &cfg, tc.as_ref(), env)?;
            if direnv {
                let env_path = store.materialize(oid)?;
                exe::direnv_output(&env_path, oid, &extras);
            } else {
                let dest = match path {
                    Some(p) => {
                        let p = PathBuf::from(p);
                        store.materialize_to(oid, &p)?;
                        p
                    }
                    None => store.materialize(oid)?,
                };
                println!("{}", dest.display());
            }
        }

        Command::Gc => {
            let stats = store.gc()?;
            println!("store entries: {}", stats.store_entries);
            println!("blobs:         {}", stats.blobs);
            println!("runs:          {}", stats.runs);
        }

        Command::Status => {
            print_status(&store)?;
        }

        Command::Track {
            name,
            source,
            strip_prefix,
            toolchains,
        } => {
            let tc_path = PathBuf::from(&toolchains);
            let mut tc = if tc_path.exists() {
                load_toolchains(&tc_path)?
            } else {
                ToolchainsConfig::default()
            };

            // If source looks like a tree OID, just record it directly.
            let oid = if git2::Oid::from_str(&source).is_ok() {
                source.clone()
            } else {
                // Treat as a path — import as tarball or directory.
                let path = PathBuf::from(&source);
                if path.is_dir() {
                    import_dir(&store, &path)?.to_string()
                } else {
                    import_tarball(&store, &path, strip_prefix)?.to_string()
                }
            };

            tc.toolchains.insert(
                name.clone(),
                ToolchainDef {
                    source,
                    oid: Some(oid.clone()),
                    strip_prefix,
                },
            );
            save_toolchains(&tc_path, &tc)?;
            println!("{name} = {oid}");
        }

        Command::Untrack { name, toolchains } => {
            let tc_path = PathBuf::from(&toolchains);
            let mut tc = load_toolchains(&tc_path)?;
            if tc.toolchains.remove(&name).is_none() {
                return Err(Error::Config(format!(
                    "toolchain '{name}' not found in {toolchains}"
                )));
            }
            save_toolchains(&tc_path, &tc)?;
        }
    }

    Ok(())
}

fn resolve_oid(
    store: &Store,
    config: &hearth::env::Config,
    toolchains: Option<&ToolchainsConfig>,
    s: &str,
) -> Result<git2::Oid, Error> {
    if let Ok(oid) = git2::Oid::from_str(s) {
        return Ok(oid);
    }
    resolve_env(store, config, toolchains, s)
}

fn print_diff(repo: &git2::Repository, oid_a: git2::Oid, oid_b: git2::Oid) -> Result<(), Error> {
    let tree_a = repo.find_tree(oid_a)?;
    let tree_b = repo.find_tree(oid_b)?;
    let diff = repo.diff_tree_to_tree(Some(&tree_a), Some(&tree_b), None)?;
    diff.print(git2::DiffFormat::Patch, |_delta, _hunk, line| {
        let origin = line.origin();
        if matches!(origin, '+' | '-' | ' ') {
            print!("{origin}");
        }
        if let Ok(s) = std::str::from_utf8(line.content()) {
            print!("{s}");
        }
        true
    })?;
    Ok(())
}

fn print_status(store: &Store) -> Result<(), Error> {
    let trees = store.list_trees()?;
    let envs = store.list_envs()?;
    println!("trees: {}", trees.len());
    println!("envs:  {}", envs.len());

    let blobs_dir = store.root().join("blobs");
    let mut blob_count = 0u64;
    let mut blob_bytes = 0u64;
    if blobs_dir.exists() {
        for entry in std::fs::read_dir(&blobs_dir)? {
            let entry = entry?;
            blob_bytes += entry.metadata()?.len();
            blob_count += 1;
        }
    }
    println!("blobs: {blob_count} ({blob_bytes} bytes cached)");
    Ok(())
}
