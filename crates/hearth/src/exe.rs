//! Environment activation: enter a shell, or emit direnv-compatible output.

use std::fs;
use std::process;

use git2::Oid;

use crate::{Error, store::Store};

/// Isolation level for environment activation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[repr(u8)]
pub enum Isolation {
    /// Convention only: PATH prepended, no filesystem enforcement.
    #[default]
    Workspace = 0,
    /// Read-only inputs: store chmod'd read-only, writes captured per-run.
    ReadOnly = 1,
    /// Filesystem isolation: store mounted read-only, writes captured per-run.
    Filesystem = 2,
    /// Network isolation: read-only mounts, writes captures, and no access to the network.
    Network = 3,
}

impl Isolation {
    /// Convert from a numeric level.
    ///
    /// Convert from a numeric isolation level.
    pub fn from_u8(n: u8) -> Result<Self, Error> {
        match n {
            0 => Ok(Self::Workspace),
            1 => Ok(Self::ReadOnly),
            _ => todo!("isolation level {n} requires VM support"),
        }
    }
}

/// Enter an environment by spawning a shell inside it.
///
/// Returns the exit status of the spawned shell.
pub fn enter(
    store: &Store,
    tree_oid: Oid,
    isolation: Isolation,
) -> Result<process::ExitStatus, Error> {
    let store_path = store.materialize(tree_oid)?;

    if isolation == Isolation::ReadOnly {
        let id = run_id();
        let capture = store.root().join("runs").join(&id).join("capture");
        fs::create_dir_all(&capture)?;
        set_read_only_recursive(&store_path)?;
        spawn_shell(&store_path, Some(&capture), tree_oid, isolation)
    } else {
        spawn_shell(&store_path, None, tree_oid, isolation)
    }
}

/// Print shell-eval-able direnv output for an environment tree.
///
/// Prepends `<tree>/bin` to PATH and exports `HEARTH_ENV`.
pub fn direnv_output(env_path: &std::path::Path, tree_oid: Oid) {
    let bin = env_path.join("bin");
    let existing_path = std::env::var("PATH").unwrap_or_default();
    if existing_path.is_empty() {
        println!("export PATH=\"{}\"", bin.display());
    } else {
        println!("export PATH=\"{}:{}\"", bin.display(), existing_path);
    }
    println!("export HEARTH_ENV=\"{tree_oid}\"");
    println!("export HEARTH_ISOLATION=\"0\"");
}

fn spawn_shell(
    env_path: &std::path::Path,
    capture: Option<&std::path::Path>,
    tree_oid: Oid,
    isolation: Isolation,
) -> Result<process::ExitStatus, Error> {
    let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".into());

    let mut cmd = process::Command::new(&shell);
    cmd.env("HEARTH_ENV", tree_oid.to_string());
    cmd.env("HEARTH_ISOLATION", (isolation as u8).to_string());

    prepend_path(&mut cmd, &env_path.join("bin"));

    if let Some(capture_dir) = capture {
        cmd.env("TMPDIR", capture_dir);
    }

    Ok(cmd.status()?)
}

fn prepend_path(cmd: &mut process::Command, dir: &std::path::Path) {
    let existing = std::env::var("PATH").unwrap_or_default();
    if existing.is_empty() {
        cmd.env("PATH", dir.display().to_string());
    } else {
        cmd.env("PATH", format!("{}:{}", dir.display(), existing));
    }
}

fn run_id() -> String {
    let t = std::time::SystemTime::now()
        .duration_since(std::time::SystemTime::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    format!("{t:x}-{}", process::id())
}

#[cfg(unix)]
fn set_read_only_recursive(path: &std::path::Path) -> Result<(), Error> {
    use std::os::unix::fs::PermissionsExt;
    for entry in fs::read_dir(path)? {
        let entry = entry?;
        let meta = entry.metadata()?;
        let p = entry.path();
        if meta.is_dir() {
            set_read_only_recursive(&p)?;
        } else if meta.is_file() {
            let mut perms = meta.permissions();
            let mode = perms.mode() & !0o222;
            perms.set_mode(mode);
            fs::set_permissions(&p, perms)?;
        }
    }
    Ok(())
}

#[cfg(not(unix))]
fn set_read_only_recursive(_path: &std::path::Path) -> Result<(), Error> {
    Ok(())
}
