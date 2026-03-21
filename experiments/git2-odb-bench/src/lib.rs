//! Shared helpers for git-kiln integration benchmarks.
//!
//! Provides repository setup, artifact generation, and measurement utilities
//! used by the benchmark binaries.

pub mod artifact;

use std::path::Path;

use git2::{Repository, Signature};
use tempfile::TempDir;

/// A bare repository in a temp directory, ready for benchmarking.
pub struct BenchRepo {
    /// Keep the `TempDir` alive so the directory isn't cleaned up.
    pub dir: TempDir,
    /// The bare git repository.
    pub repo: Repository,
}

impl BenchRepo {
    /// Create a bare repo with `core.looseCompression` set to `level`.
    pub fn new(compression_level: i32) -> Self {
        let dir = TempDir::new().expect("tempdir");
        let repo = Repository::init_bare(dir.path()).expect("init bare repo");
        repo.config()
            .expect("repo config")
            .set_i32("core.looseCompression", compression_level)
            .expect("set looseCompression");
        Self { dir, repo }
    }

    /// Total on-disk size of the repository in bytes.
    pub fn size_bytes(&self) -> u64 {
        dir_size_bytes(self.dir.path())
    }
}

/// Return a reusable committer signature.
pub fn bench_sig() -> Signature<'static> {
    Signature::now("kiln-bench", "bench@kiln").expect("signature")
}

/// Recursively compute directory size.
pub fn dir_size_bytes(path: &Path) -> u64 {
    let mut total = 0u64;
    if let Ok(entries) = std::fs::read_dir(path) {
        for entry in entries.flatten() {
            if let Ok(m) = entry.metadata() {
                if m.is_dir() {
                    total += dir_size_bytes(&entry.path());
                } else {
                    total += m.len();
                }
            }
        }
    }
    total
}
