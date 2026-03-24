//! `forge run` — enter a hearth environment.

use std::path::PathBuf;
use std::process;

use hearth::{
    env::{load_config, resolve_env, resolve_extras},
    exe::{self, Isolation},
    store::Store,
};

pub(crate) fn run(
    env: &str,
    isolation: u8,
    config: &str,
    store_path: Option<&str>,
    command: &[String],
) {
    if let Err(e) = run_inner(env, isolation, config, store_path, command) {
        eprintln!("error: {e}");
        process::exit(1);
    }
}

fn run_inner(
    env: &str,
    isolation: u8,
    config: &str,
    store_path: Option<&str>,
    command: &[String],
) -> Result<(), hearth::Error> {
    let store = match store_path {
        Some(p) => Store::open_or_init(&PathBuf::from(p))?,
        None => Store::open_default()?,
    };

    let cfg = load_config(&PathBuf::from(config))?;
    let extras = resolve_extras(&cfg, env)?;
    let oid = resolve_env(&store, &cfg, env)?;
    let level = Isolation::from_u8(isolation)?;

    if command.is_empty() {
        let status = exe::enter(&store, oid, level, &extras)?;
        process::exit(status.code().unwrap_or(1));
    }

    // Run a specific command instead of an interactive shell.
    let store_path = store.materialize(oid)?;
    let bin = store_path.join("bin");

    let mut cmd = process::Command::new(&command[0]);
    cmd.args(&command[1..]);
    cmd.env("HEARTH_ENV", oid.to_string());
    cmd.env("HEARTH_ISOLATION", isolation.to_string());

    match level {
        Isolation::Host => {}
        Isolation::Workspace | Isolation::ReadOnly => {
            let mut parts = vec![bin.display().to_string()];
            parts.extend(extras.iter().cloned());
            cmd.env("PATH", parts.join(":"));
        }
    }

    let status = cmd.status().map_err(hearth::Error::from)?;
    process::exit(status.code().unwrap_or(1));
}
