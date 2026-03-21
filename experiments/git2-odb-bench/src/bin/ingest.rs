//! Benchmark 1: Ingestion Throughput
//!
//! Measures how fast we can write N blobs into the ODB and construct a tree,
//! across a matrix of compression levels and artifact set sizes.
//!
//! Run: `cargo run --release -p git-kiln-bench --bin ingest [-- OPTIONS]`
//!
//! Options:
//!   --crates 5,10      Comma-separated crate counts (default: 30,100,300)
//!   --levels 0,6       Comma-separated compression levels (default: 0,1,6)
//!   --iterations 2     Iterations per cell (default: 3)

#![allow(missing_docs)]

use std::collections::BTreeMap;
use std::env;
use std::time::Instant;

use git2::FileMode;
use git2_odb_bench::artifact::{self, BuildArtifacts};
use git2_odb_bench::{BenchRepo, bench_sig};
use rand::SeedableRng;
use rand::rngs::StdRng;

/// "KILN_ING" as a little-endian u64.
const SEED: u64 = 0x4b494c4e5f494e47;

fn parse_csv<T: std::str::FromStr>(s: &str) -> Vec<T> {
    s.split(',')
        .filter_map(|v| v.trim().parse().ok())
        .collect()
}

fn get_arg(name: &str) -> Option<String> {
    let args: Vec<String> = env::args().collect();
    args.iter()
        .position(|a| a == name)
        .and_then(|i| args.get(i + 1).cloned())
}

struct IngestResult {
    wall_time_ms: f64,
    throughput_mb_s: f64,
    repo_size_mb: f64,
}

/// Ingest all artifacts into a bare repo as blobs, build a nested tree, and
/// commit. Returns the measurement.
fn run_ingest(compression_level: i32, artifacts: &BuildArtifacts) -> IngestResult {
    let bench_repo = BenchRepo::new(compression_level);
    let repo = &bench_repo.repo;
    let sig = bench_sig();

    let start = Instant::now();

    // Group artifacts by their top-level directory for nested tree construction.
    let mut subtrees: BTreeMap<String, Vec<(String, git2::Oid)>> = BTreeMap::new();

    for artifact in &artifacts.artifacts {
        let oid = repo.blob(&artifact.data).expect("blob write");

        // Split path into directory prefix and filename.
        if let Some((dir, name)) = artifact.path.rsplit_once('/') {
            // Handle nested paths (e.g. `build/crate_0000/out/generated.rs`).
            // We flatten into the first directory component for the top-level tree
            // and store the rest as a compound name to build intermediate trees.
            let top = dir.split('/').next().unwrap_or(dir);
            let remainder = if dir.contains('/') {
                format!("{}/{name}", &dir[top.len() + 1..])
            } else {
                name.to_string()
            };
            subtrees
                .entry(top.to_string())
                .or_default()
                .push((remainder, oid));
        } else {
            subtrees
                .entry(String::new())
                .or_default()
                .push((artifact.path.clone(), oid));
        }
    }

    // Build the tree hierarchy. For simplicity, we create one level of subtrees
    // for the top-level directories and flatten deeper paths as blob names.
    // This matches how Kiln would actually store outputs (deps/, build/, etc).
    let mut root_tb = repo.treebuilder(None).expect("root treebuilder");

    for (dir, entries) in &subtrees {
        if dir.is_empty() {
            for (name, oid) in entries {
                root_tb
                    .insert(name, *oid, FileMode::Blob.into())
                    .expect("insert root blob");
            }
        } else {
            // Build nested trees for paths with multiple components.
            let subtree_oid = build_nested_tree(repo, entries);
            root_tb
                .insert(dir, subtree_oid, FileMode::Tree.into())
                .expect("insert subtree");
        }
    }

    let tree_oid = root_tb.write().expect("write root tree");
    let tree = repo.find_tree(tree_oid).expect("find tree");
    repo.commit(None, &sig, &sig, "ingest benchmark", &tree, &[])
        .expect("commit");

    let elapsed = start.elapsed();
    let elapsed_secs = elapsed.as_secs_f64();
    let total_mb = artifacts.total_bytes as f64 / 1_048_576.0;

    IngestResult {
        wall_time_ms: elapsed.as_secs_f64() * 1000.0,
        throughput_mb_s: total_mb / elapsed_secs,
        repo_size_mb: bench_repo.size_bytes() as f64 / 1_048_576.0,
    }
}

/// Recursively build nested trees from a flat list of `(relative_path, oid)` entries.
fn build_nested_tree(repo: &git2::Repository, entries: &[(String, git2::Oid)]) -> git2::Oid {
    let mut direct: Vec<(&str, git2::Oid)> = Vec::new();
    let mut subdirs: BTreeMap<&str, Vec<(String, git2::Oid)>> = BTreeMap::new();

    for (path, oid) in entries {
        if let Some((dir, rest)) = path.split_once('/') {
            subdirs
                .entry(dir)
                .or_default()
                .push((rest.to_string(), *oid));
        } else {
            direct.push((path.as_str(), *oid));
        }
    }

    let mut tb = repo.treebuilder(None).expect("treebuilder");

    for (name, oid) in &direct {
        tb.insert(name, *oid, FileMode::Blob.into())
            .expect("insert blob");
    }

    for (dir, sub_entries) in &subdirs {
        let sub_oid = build_nested_tree(repo, sub_entries);
        tb.insert(dir, sub_oid, FileMode::Tree.into())
            .expect("insert subtree");
    }

    tb.write().expect("write tree")
}

fn main() {
    let compression_levels: Vec<i32> =
        get_arg("--levels").map_or(vec![0, 1, 6], |s| parse_csv(&s));
    let crate_counts: Vec<usize> =
        get_arg("--crates").map_or(vec![30, 100, 300], |s| parse_csv(&s));
    let iterations: usize = get_arg("--iterations")
        .and_then(|s| s.parse().ok())
        .unwrap_or(3);

    println!("Benchmark 1: Ingestion Throughput");
    println!("=================================");
    println!("Iterations per cell: {iterations}");
    println!();

    // Pre-generate artifact sets (seeded for reproducibility).
    let mut artifact_sets: Vec<(usize, BuildArtifacts)> = Vec::new();
    for &n in &crate_counts {
        let mut rng = StdRng::seed_from_u64(SEED.wrapping_add(n as u64));
        let build = artifact::generate_build(&mut rng, n);
        println!(
            "  artifact set: {n:>3} crates, {files:>4} files, {mb:>7.1} MB",
            files = build.artifacts.len(),
            mb = build.total_bytes as f64 / 1_048_576.0,
        );
        artifact_sets.push((n, build));
    }
    println!();

    // Header
    println!(
        "{:>6} {:>6} {:>5} {:>10} {:>10} {:>10}",
        "crates", "zlib", "iter", "time(ms)", "MB/s", "repo(MB)"
    );
    println!("{}", "-".repeat(60));

    // Success criterion from design doc: >100 MB/s at compression level 0.
    let mut pass_fail: Vec<(usize, i32, f64)> = Vec::new();

    for &level in &compression_levels {
        for (n, artifacts) in &artifact_sets {
            for iter in 1..=iterations {
                let result = run_ingest(level, artifacts);
                println!(
                    "{n:>6} {level:>6} {iter:>5} {time:>10.1} {tp:>10.1} {sz:>10.1}",
                    time = result.wall_time_ms,
                    tp = result.throughput_mb_s,
                    sz = result.repo_size_mb,
                );
                if iter == iterations {
                    pass_fail.push((*n, level, result.throughput_mb_s));
                }
            }
        }
    }

    println!();
    println!("Success criteria: >100 MB/s at compression level 0");
    println!();
    for (n, level, tp) in &pass_fail {
        if *level == 0 {
            let status = if *tp > 100.0 { "PASS" } else { "FAIL" };
            println!("  {n:>3} crates, level {level}: {tp:>7.1} MB/s  [{status}]");
        }
    }
}
