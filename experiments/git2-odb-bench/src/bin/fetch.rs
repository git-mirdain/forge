//! Benchmark 4: Fetch / Restore Simulation
//!
//! Measures how fast a client can restore a full build's outputs from a local
//! remote. Uses a bare repo as the "remote" and fetches into a separate client
//! repo over `file://` — this is the realistic path for Kiln cache hits.
//!
//! Run: `cargo run --release -p git-kiln-bench --bin fetch [-- OPTIONS]`
//!
//! Options:
//!   --crates 30        Crates per build (default: 30)
//!   --iterations 3     Iterations (default: 3)

#![allow(missing_docs)]

use std::collections::BTreeMap;
use std::io::Write;
use std::path::Path;
use std::process::Command;
use std::time::Instant;

use git2::{FileMode, Repository};
use git2_odb_bench::artifact::{self, BuildArtifacts};
use git2_odb_bench::{BenchRepo, bench_sig};
use rand::SeedableRng;
use rand::rngs::StdRng;
use tempfile::TempDir;

const SEED: u64 = 0x4b494c4e5f465443;

fn get_arg(name: &str) -> Option<String> {
    let args: Vec<String> = std::env::args().collect();
    args.iter()
        .position(|a| a == name)
        .and_then(|i| args.get(i + 1).cloned())
}

/// Ingest artifacts into the repo under a single commit, return the ref name.
fn ingest_build(repo: &Repository, artifacts: &BuildArtifacts, ref_name: &str) {
    let sig = bench_sig();
    let mut subtrees: BTreeMap<String, Vec<(String, git2::Oid)>> = BTreeMap::new();

    for artifact in &artifacts.artifacts {
        let oid = repo.blob(&artifact.data).expect("blob write");
        if let Some((dir, name)) = artifact.path.rsplit_once('/') {
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

    let mut root_tb = repo.treebuilder(None).expect("root treebuilder");
    for (dir, entries) in &subtrees {
        if dir.is_empty() {
            for (name, oid) in entries {
                root_tb
                    .insert(name, *oid, FileMode::Blob.into())
                    .expect("insert blob");
            }
        } else {
            let sub_oid = build_nested_tree(repo, entries);
            root_tb
                .insert(dir, sub_oid, FileMode::Tree.into())
                .expect("insert subtree");
        }
    }

    let tree_oid = root_tb.write().expect("write root tree");
    let tree = repo.find_tree(tree_oid).expect("find tree");
    let commit_oid = repo
        .commit(None, &sig, &sig, "build output", &tree, &[])
        .expect("commit");
    repo.reference(ref_name, commit_oid, true, "ingest")
        .expect("create ref");
}

fn build_nested_tree(repo: &Repository, entries: &[(String, git2::Oid)]) -> git2::Oid {
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

/// Walk a tree and write all blobs to `dest`. Returns total bytes written.
fn materialize(repo: &Repository, tree: &git2::Tree<'_>, dest: &Path) -> u64 {
    let mut bytes = 0u64;
    for entry in tree.iter() {
        let name = entry.name().unwrap_or("_");
        let path = dest.join(name);
        match entry.kind() {
            Some(git2::ObjectType::Tree) => {
                std::fs::create_dir_all(&path).expect("mkdir");
                let sub = repo.find_tree(entry.id()).expect("find subtree");
                bytes += materialize(repo, &sub, &path);
            }
            Some(git2::ObjectType::Blob) => {
                if let Some(parent) = path.parent() {
                    std::fs::create_dir_all(parent).ok();
                }
                let blob = repo.find_blob(entry.id()).expect("find blob");
                let mut f = std::fs::File::create(&path).expect("create file");
                f.write_all(blob.content()).expect("write blob");
                bytes += blob.content().len() as u64;
            }
            _ => {}
        }
    }
    bytes
}

struct FetchResult {
    transfer_ms: f64,
    materialize_ms: f64,
    total_ms: f64,
    bytes: u64,
    throughput_mb_s: f64,
}

/// `git fetch` over `file://` protocol — includes full packfile negotiation.
fn run_git_fetch(
    remote_path: &Path,
    ref_name: &str,
    out_dir: &Path,
    extra_args: &[&str],
) -> FetchResult {
    let total_start = Instant::now();

    let client_dir = TempDir::new().expect("client tempdir");
    let fetch_start = Instant::now();

    let remote_url = format!("file://{}", remote_path.display());
    let refspec = format!("+{ref_name}:{ref_name}");

    let output = Command::new("git")
        .args(["init", "--bare"])
        .arg(client_dir.path())
        .output()
        .expect("git init");
    assert!(output.status.success(), "git init failed");

    let mut fetch_cmd = Command::new("git");
    fetch_cmd.args(["fetch", "--no-tags"]);
    fetch_cmd.args(extra_args);
    fetch_cmd.args([remote_url.as_str(), refspec.as_str()]);
    fetch_cmd.current_dir(client_dir.path());

    let output = fetch_cmd.output().expect("git fetch");
    assert!(
        output.status.success(),
        "git fetch failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let transfer_ms = fetch_start.elapsed().as_secs_f64() * 1000.0;

    let client_repo = Repository::open_bare(client_dir.path()).expect("open client repo");
    let mat_start = Instant::now();
    let reference = client_repo.find_reference(ref_name).expect("find ref");
    let commit = reference.peel_to_commit().expect("peel commit");
    let tree = commit.tree().expect("tree");
    let bytes = materialize(&client_repo, &tree, out_dir);
    let materialize_ms = mat_start.elapsed().as_secs_f64() * 1000.0;

    let total_ms = total_start.elapsed().as_secs_f64() * 1000.0;
    let throughput_mb_s = (bytes as f64 / 1_048_576.0) / (total_ms / 1000.0);

    FetchResult {
        transfer_ms,
        materialize_ms,
        total_ms,
        bytes,
        throughput_mb_s,
    }
}

/// Direct ODB-to-ODB copy — walk the commit's tree in the remote repo,
/// copy every reachable object into the client repo's ODB, then materialize.
/// Skips packfile negotiation entirely.
fn run_odb_copy(remote_repo: &Repository, ref_name: &str, out_dir: &Path) -> FetchResult {
    let total_start = Instant::now();

    let client_dir = TempDir::new().expect("client tempdir");
    let client_repo = Repository::init_bare(client_dir.path()).expect("init client");

    let reference = remote_repo.find_reference(ref_name).expect("find ref");
    let commit = reference.peel_to_commit().expect("peel commit");

    let copy_start = Instant::now();

    // Copy the commit object.
    let remote_odb = remote_repo.odb().expect("remote odb");
    let client_odb = client_repo.odb().expect("client odb");

    let commit_obj = remote_odb.read(commit.id()).expect("read commit");
    client_odb
        .write(commit_obj.kind(), commit_obj.data())
        .expect("write commit");

    // Walk the tree and copy all reachable objects.
    let tree = commit.tree().expect("tree");
    copy_tree_recursive(remote_repo, &remote_odb, &client_odb, tree.id());

    let transfer_ms = copy_start.elapsed().as_secs_f64() * 1000.0;

    // Create the ref in the client repo.
    client_repo
        .reference(ref_name, commit.id(), true, "odb copy")
        .expect("create ref");

    // Materialize from the client repo.
    let mat_start = Instant::now();
    let client_ref = client_repo.find_reference(ref_name).expect("find ref");
    let client_commit = client_ref.peel_to_commit().expect("peel commit");
    let client_tree = client_commit.tree().expect("tree");
    let bytes = materialize(&client_repo, &client_tree, out_dir);
    let materialize_ms = mat_start.elapsed().as_secs_f64() * 1000.0;

    let total_ms = total_start.elapsed().as_secs_f64() * 1000.0;
    let throughput_mb_s = (bytes as f64 / 1_048_576.0) / (total_ms / 1000.0);

    FetchResult {
        transfer_ms,
        materialize_ms,
        total_ms,
        bytes,
        throughput_mb_s,
    }
}

fn copy_tree_recursive(
    remote_repo: &Repository,
    remote_odb: &git2::Odb<'_>,
    client_odb: &git2::Odb<'_>,
    tree_oid: git2::Oid,
) {
    let tree_obj = remote_odb.read(tree_oid).expect("read tree");
    client_odb
        .write(tree_obj.kind(), tree_obj.data())
        .expect("write tree");

    let tree = remote_repo.find_tree(tree_oid).expect("find tree");
    for entry in tree.iter() {
        match entry.kind() {
            Some(git2::ObjectType::Tree) => {
                copy_tree_recursive(remote_repo, remote_odb, client_odb, entry.id());
            }
            Some(git2::ObjectType::Blob) => {
                let blob_obj = remote_odb.read(entry.id()).expect("read blob");
                client_odb
                    .write(blob_obj.kind(), blob_obj.data())
                    .expect("write blob");
            }
            _ => {}
        }
    }
}

/// Git bundle strategy — simulates the network path:
/// 1. Server creates a bundle (pre-built packfile) for the ref.
/// 2. Client "downloads" it (reads from disk — substitute for HTTP GET).
/// 3. Client unbundles into its ODB and materializes.
///
/// This is the realistic network-analogous path: one request, one packfile,
/// no negotiation. The bundle creation would happen at CI push time.
fn run_bundle(remote_path: &Path, ref_name: &str, out_dir: &Path) -> FetchResult {
    // Phase 0 (server-side, not timed): create the bundle.
    // In production this happens once at CI push time, not per-fetch.
    let bundle_file = TempDir::new().expect("bundle tempdir");
    let bundle_path = bundle_file.path().join("output.bundle");

    let output = Command::new("git")
        .args(["bundle", "create", bundle_path.to_str().unwrap(), ref_name])
        .current_dir(remote_path)
        .output()
        .expect("git bundle create");
    assert!(
        output.status.success(),
        "git bundle create failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let bundle_size = std::fs::metadata(&bundle_path)
        .map(|m| m.len())
        .unwrap_or(0);

    // Phase 1 (timed): simulate "download" by reading the bundle into memory,
    // then unbundle into a fresh client repo.
    let client_dir = TempDir::new().expect("client tempdir");

    let output = Command::new("git")
        .args(["init", "--bare"])
        .arg(client_dir.path())
        .output()
        .expect("git init");
    assert!(output.status.success(), "git init failed");

    let total_start = Instant::now();
    let transfer_start = Instant::now();

    // Read bundle into memory (simulates network transfer).
    let bundle_bytes = std::fs::read(&bundle_path).expect("read bundle");
    std::hint::black_box(&bundle_bytes);

    // Write to client-local temp (simulates writing to disk after download).
    let client_bundle = client_dir.path().join("incoming.bundle");
    std::fs::write(&client_bundle, &bundle_bytes).expect("write bundle");

    // Unbundle.
    let output = Command::new("git")
        .args([
            "bundle",
            "unbundle",
            client_bundle.to_str().unwrap(),
        ])
        .current_dir(client_dir.path())
        .output()
        .expect("git bundle unbundle");
    assert!(
        output.status.success(),
        "git bundle unbundle failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    // Create the ref from the unbundled objects.
    let unbundle_stdout = String::from_utf8_lossy(&output.stdout);
    let commit_hex = unbundle_stdout
        .lines()
        .next()
        .and_then(|l| l.split_whitespace().next())
        .expect("parse commit OID from unbundle output");

    let client_repo = Repository::open_bare(client_dir.path()).expect("open client repo");
    let commit_oid = git2::Oid::from_str(commit_hex).expect("parse oid");
    client_repo
        .reference(ref_name, commit_oid, true, "unbundle")
        .expect("create ref");

    let transfer_ms = transfer_start.elapsed().as_secs_f64() * 1000.0;

    // Phase 2: materialize.
    let mat_start = Instant::now();
    let reference = client_repo.find_reference(ref_name).expect("find ref");
    let commit = reference.peel_to_commit().expect("peel commit");
    let tree = commit.tree().expect("tree");
    let bytes = materialize(&client_repo, &tree, out_dir);
    let materialize_ms = mat_start.elapsed().as_secs_f64() * 1000.0;

    let total_ms = total_start.elapsed().as_secs_f64() * 1000.0;
    let throughput_mb_s = (bytes as f64 / 1_048_576.0) / (total_ms / 1000.0);

    eprintln!(
        "    bundle size: {:.1} MB (compression ratio: {:.1}x)",
        bundle_size as f64 / 1_048_576.0,
        bytes as f64 / bundle_size as f64,
    );

    FetchResult {
        transfer_ms,
        materialize_ms,
        total_ms,
        bytes,
        throughput_mb_s,
    }
}

fn main() {
    let num_crates: usize = get_arg("--crates")
        .and_then(|s| s.parse().ok())
        .unwrap_or(30);
    let iterations: usize = get_arg("--iterations")
        .and_then(|s| s.parse().ok())
        .unwrap_or(3);

    println!("Benchmark 4: Fetch / Restore (local remote)");
    println!("============================================");
    println!("Crates: {num_crates}, Iterations: {iterations}");
    println!();

    // Pre-generate artifacts and ingest into a bare "remote" repo.
    let mut rng = StdRng::seed_from_u64(SEED.wrapping_add(num_crates as u64));
    let build = artifact::generate_build(&mut rng, num_crates);
    println!(
        "  build: {} files, {:.1} MB",
        build.artifacts.len(),
        build.total_bytes as f64 / 1_048_576.0,
    );

    let remote_repo = BenchRepo::new(0);
    let ref_name = "refs/kiln/outputs/bench_fetch";
    print!("  ingesting into remote repo...");
    std::io::stdout().flush().ok();
    ingest_build(&remote_repo.repo, &build, ref_name);
    println!(
        " done ({:.1} MB on disk)",
        remote_repo.size_bytes() as f64 / 1_048_576.0
    );
    println!();

    let header = format!(
        "{:>5} {:>12} {:>12} {:>10} {:>10} {:>10}",
        "iter", "transfer(ms)", "materialize", "total(ms)", "bytes(MB)", "MB/s"
    );
    let sep = "-".repeat(67);

    let run_strategy = |label: &str, f: &dyn Fn(usize) -> FetchResult| {
        println!("Strategy: {label}");
        println!("{header}");
        println!("{sep}");
        for iter in 1..=iterations {
            let result = f(iter);
            println!(
                "{iter:>5} {xfer:>12.1} {mat:>10.1}ms {total:>10.1} {bytes:>10.1} {tp:>10.1}",
                xfer = result.transfer_ms,
                mat = result.materialize_ms,
                total = result.total_ms,
                bytes = result.bytes as f64 / 1_048_576.0,
                tp = result.throughput_mb_s,
            );
        }
        println!();
    };

    // --- 1: git fetch (loose objects remote) ---
    run_strategy(
        "git fetch (loose remote, full negotiation)",
        &|_| {
            let out_dir = TempDir::new().expect("tempdir");
            run_git_fetch(remote_repo.dir.path(), ref_name, out_dir.path(), &[])
        },
    );

    // --- 2: git fetch --depth=1 (loose objects remote) ---
    run_strategy(
        "git fetch --depth=1 (loose remote, shallow)",
        &|_| {
            let out_dir = TempDir::new().expect("tempdir");
            run_git_fetch(
                remote_repo.dir.path(),
                ref_name,
                out_dir.path(),
                &["--depth=1"],
            )
        },
    );

    // --- 3: direct ODB copy ---
    run_strategy(
        "direct ODB copy (walk tree, copy objects by OID)",
        &|_| {
            let out_dir = TempDir::new().expect("tempdir");
            run_odb_copy(&remote_repo.repo, ref_name, out_dir.path())
        },
    );

    // --- 4: git bundle (pre-built packfile) ---
    run_strategy(
        "git bundle (pre-built packfile, simulates HTTP download)",
        &|_| {
            let out_dir = TempDir::new().expect("tempdir");
            run_bundle(remote_repo.dir.path(), ref_name, out_dir.path())
        },
    );

    // --- Repack the remote, then re-run fetch strategies ---
    print!("  repacking remote...");
    std::io::stdout().flush().ok();
    let output = Command::new("git")
        .args(["repack", "-a", "-d", "-f"])
        .current_dir(remote_repo.dir.path())
        .output()
        .expect("git repack");
    assert!(
        output.status.success(),
        "git repack failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    println!(
        " done ({:.1} MB on disk)",
        remote_repo.size_bytes() as f64 / 1_048_576.0
    );
    println!();

    // --- 5: git fetch (packed remote) ---
    run_strategy(
        "git fetch (packed remote, full negotiation)",
        &|_| {
            let out_dir = TempDir::new().expect("tempdir");
            run_git_fetch(remote_repo.dir.path(), ref_name, out_dir.path(), &[])
        },
    );

    // --- 6: git fetch --depth=1 (packed remote) ---
    run_strategy(
        "git fetch --depth=1 (packed remote, shallow)",
        &|_| {
            let out_dir = TempDir::new().expect("tempdir");
            run_git_fetch(
                remote_repo.dir.path(),
                ref_name,
                out_dir.path(),
                &["--depth=1"],
            )
        },
    );

    println!("Success criteria: <5s for 280MB output tree from local remote");
}
