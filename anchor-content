//! PID-file guard: prevents two forge-server instances for the same repo.

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Result, bail};

/// A guard that removes the PID file when dropped.
pub struct PidGuard {
    path: PathBuf,
}

impl PidGuard {
    /// Acquire the PID file at `git_dir/forge-server.pid`.
    ///
    /// If a PID file already exists and the process is still alive, returns an
    /// error. Stale PID files (process no longer running) are reclaimed.
    pub fn acquire(git_dir: &Path) -> Result<Self> {
        let path = git_dir.join("forge-server.pid");

        let me = std::process::id();
        if let Ok(contents) = fs::read_to_string(&path)
            && let Ok(pid) = contents.trim().parse::<u32>()
            && pid != me
            && process_alive(pid)
        {
            bail!(
                "forge-server already running (pid {pid}) for this repository. \
                 Remove {} if the process is stale.",
                path.display()
            );
        }

        fs::write(&path, std::process::id().to_string())?;
        Ok(Self { path })
    }
}

impl Drop for PidGuard {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}

/// Check whether a process with the given PID is alive.
fn process_alive(pid: u32) -> bool {
    std::process::Command::new("kill")
        .args(["-0", &pid.to_string()])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .is_ok_and(|s| s.success())
}
