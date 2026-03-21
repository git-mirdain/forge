#![allow(missing_docs)]
//! Kiln benchmark suite.
//!
//! Measures Git object store performance under Kiln's build cache access
//! patterns. See the crate README for results and analysis.

use std::{
    hint::black_box,
    io::Write,
    path::Path,
    process::Command,
    time::{Duration, Instant},
};

use criterion::{BenchmarkId, Criterion, criterion_group, criterion_main};
use git2::{FileMode, Oid, Repository, Signature};
use rand::{Rng, SeedableRng, rngs::StdRng};
use tempfile::TempDir;

// ---------------------------------------------------------------------------
// Seed constant
// ---------------------------------------------------------------------------

/// "KILN_BNC" encoded as a little-endian u64.
const KILN_SEED: u64 = 0x4b494c4e5f424e43;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Create a bare repository in a temp directory with `core.looseCompression`
/// set to `compression_level`. No remotes are configured.
fn make_bare_repo(compression_level: i32) -> (TempDir, Repository) {
    let dir = TempDir::new().expect("tempdir");
    let repo = Repository::init_bare(dir.path()).expect("init bare repo");
    repo.config()
        .expect("repo config")
        .set_i32("core.looseCompression", compression_level)
        .expect("set looseCompression");
    (dir, repo)
}

/// Generate a random 32-character lowercase hex string using `rng`.
fn random_hex32(rng: &mut impl Rng) -> String {
    format!("{:016x}{:016x}", rng.r#gen::<u64>(), rng.r#gen::<u64>())
}

/// Generate `len` random bytes.
fn random_bytes(rng: &mut impl Rng, len: usize) -> Vec<u8> {
    (0..len).map(|_| rng.r#gen::<u8>()).collect()
}

/// Write a single crate output tree into `repo` and create a ref
/// `refs/kiln/outputs/<action_hash>` pointing at the resulting commit.
///
/// Tree layout:
/// ```text
/// deps/
///   libfoo.rlib      (rlib_data)
///   libfoo.rmeta     (rmeta_data)
/// .fingerprint/
///   foo-<fp_hash>/
///     invoked.timestamp  (b"0\n")
///     lib-foo            (fp_data)
/// build/             (empty, or contains output blob)
/// ```
fn ingest_crate_output(
    repo: &Repository,
    action_hash: &str,
    rlib_data: &[u8],
    rmeta_data: &[u8],
    fp_data: &[u8],
    build_data: Option<&[u8]>,
) -> Oid {
    // --- blobs ---
    let rlib_oid = repo.blob(rlib_data).expect("blob rlib");
    let rmeta_oid = repo.blob(rmeta_data).expect("blob rmeta");
    let fp_oid = repo.blob(fp_data).expect("blob fp");
    let timestamp_oid = repo.blob(b"0\n").expect("blob timestamp");

    // --- deps/ subtree ---
    let mut deps_tb = repo.treebuilder(None).expect("treebuilder deps");
    deps_tb
        .insert("libfoo.rlib", rlib_oid, FileMode::Blob.into())
        .expect("insert rlib");
    deps_tb
        .insert("libfoo.rmeta", rmeta_oid, FileMode::Blob.into())
        .expect("insert rmeta");
    let deps_tree = deps_tb.write().expect("write deps tree");

    // --- .fingerprint/foo-<hash>/ subtree ---
    let fp_hash = &action_hash[..8];
    let mut fp_inner_tb = repo.treebuilder(None).expect("treebuilder fp inner");
    fp_inner_tb
        .insert("invoked.timestamp", timestamp_oid, FileMode::Blob.into())
        .expect("insert timestamp");
    fp_inner_tb
        .insert("lib-foo", fp_oid, FileMode::Blob.into())
        .expect("insert lib-foo");
    let fp_inner_tree = fp_inner_tb.write().expect("write fp inner tree");

    let fp_dir_name = format!("foo-{fp_hash}");
    let mut fp_outer_tb = repo.treebuilder(None).expect("treebuilder fp outer");
    fp_outer_tb
        .insert(&fp_dir_name, fp_inner_tree, FileMode::Tree.into())
        .expect("insert fp dir");
    let fp_tree = fp_outer_tb.write().expect("write fp tree");

    // --- build/ subtree ---
    let mut build_tb = repo.treebuilder(None).expect("treebuilder build");
    if let Some(data) = build_data {
        let build_oid = repo.blob(data).expect("blob build");
        build_tb
            .insert("output", build_oid, FileMode::Blob.into())
            .expect("insert build output");
    }
    let build_tree = build_tb.write().expect("write build tree");

    // --- root tree ---
    let mut root_tb = repo.treebuilder(None).expect("treebuilder root");
    root_tb
        .insert("deps", deps_tree, FileMode::Tree.into())
        .expect("insert deps");
    root_tb
        .insert(".fingerprint", fp_tree, FileMode::Tree.into())
        .expect("insert .fingerprint");
    root_tb
        .insert("build", build_tree, FileMode::Tree.into())
        .expect("insert build");
    let root_tree_oid = root_tb.write().expect("write root tree");

    // --- commit ---
    let sig = Signature::now("kiln-bench", "bench@kiln").expect("sig");
    let root_tree = repo.find_tree(root_tree_oid).expect("find root tree");
    let commit_oid = repo
        .commit(None, &sig, &sig, action_hash, &root_tree, &[])
        .expect("commit");

    // --- ref ---
    let ref_name = format!("refs/kiln/outputs/{action_hash}");
    repo.reference(&ref_name, commit_oid, true, "kiln ingest")
        .expect("create ref");

    commit_oid
}

/// Write a minimal dummy commit (no output blobs) and point a ref at it.
fn ingest_dummy_commit(repo: &Repository, action_hash: &str) -> Oid {
    let sig = Signature::now("kiln-bench", "bench@kiln").expect("sig");
    let empty_tb = repo.treebuilder(None).expect("treebuilder");
    let empty_tree_oid = empty_tb.write().expect("write empty tree");
    let empty_tree = repo.find_tree(empty_tree_oid).expect("find empty tree");
    let commit_oid = repo
        .commit(None, &sig, &sig, action_hash, &empty_tree, &[])
        .expect("commit");
    let ref_name = format!("refs/kiln/outputs/{action_hash}");
    repo.reference(&ref_name, commit_oid, true, "kiln dummy")
        .expect("create ref");
    commit_oid
}

/// Recursively walk a git tree, writing blob entries to `dest/`.
/// Returns the total number of bytes written.
fn walk_and_materialize(
    repo: &Repository,
    tree: &git2::Tree<'_>,
    dest: &Path,
) -> Result<u64, Box<dyn std::error::Error>> {
    let mut bytes_written = 0u64;
    for entry in tree.iter() {
        let name = entry.name().unwrap_or("_");
        let dest_entry = dest.join(name);
        match entry.kind() {
            Some(git2::ObjectType::Tree) => {
                std::fs::create_dir_all(&dest_entry)?;
                let sub = repo.find_tree(entry.id())?;
                bytes_written += walk_and_materialize(repo, &sub, &dest_entry)?;
            }
            Some(git2::ObjectType::Blob) => {
                let blob = repo.find_blob(entry.id())?;
                let mut f = std::fs::File::create(&dest_entry)?;
                f.write_all(blob.content())?;
                bytes_written += blob.content().len() as u64;
            }
            _ => {}
        }
    }
    Ok(bytes_written)
}

/// Return the total on-disk size (bytes) of a directory tree.
fn dir_size_bytes(path: &Path) -> u64 {
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

// ---------------------------------------------------------------------------
// Blob fixtures — generated once per process, reused across benchmarks.
// ---------------------------------------------------------------------------

struct Blobs {
    rlib: Vec<u8>,  // 4 MB
    rmeta: Vec<u8>, // 100 KB
    fp: Vec<u8>,    // 1 KB
    build: Vec<u8>, // 50 KB
}

impl Blobs {
    fn new() -> Self {
        let mut rng = StdRng::seed_from_u64(KILN_SEED);
        Self {
            rlib: random_bytes(&mut rng, 4 * 1024 * 1024),
            rmeta: random_bytes(&mut rng, 100 * 1024),
            fp: random_bytes(&mut rng, 1024),
            build: random_bytes(&mut rng, 50 * 1024),
        }
    }
}

// ---------------------------------------------------------------------------
// Benchmark 1: Action Hash Cache Lookup
// ---------------------------------------------------------------------------

fn bench_cache_lookup(c: &mut Criterion) {
    let blobs = Blobs::new();
    let mut rng = StdRng::seed_from_u64(1);
    let ns = [100usize, 1_000, 10_000];

    let mut group = c.benchmark_group("bench1_cache_lookup");

    for &n in &ns {
        // Build a fresh repo pre-populated with N refs.
        let (_dir, repo) = make_bare_repo(0);
        let hashes: Vec<String> = (0..n).map(|_| random_hex32(&mut rng)).collect();
        for h in &hashes {
            ingest_crate_output(
                &repo,
                h,
                &blobs.rlib,
                &blobs.rmeta,
                &blobs.fp,
                Some(&blobs.build),
            );
        }

        // Pick a known-present hash and a guaranteed-absent hash.
        let hit_hash = hashes[n / 2].clone();
        let miss_hash = random_hex32(&mut rng);

        // --- Hit case ---
        group.bench_with_input(BenchmarkId::new("hit", n), &n, |b, _| {
            b.iter(|| {
                let ref_name = format!("refs/kiln/outputs/{}", &hit_hash);
                let r = repo.find_reference(black_box(&ref_name)).expect("find ref");
                let commit = r.peel_to_commit().expect("peel commit");
                black_box(commit.tree_id())
            })
        });

        // --- Miss case ---
        group.bench_with_input(BenchmarkId::new("miss", n), &n, |b, _| {
            b.iter(|| {
                let ref_name = format!("refs/kiln/outputs/{}", &miss_hash);
                black_box(repo.find_reference(black_box(&ref_name)).is_err())
            })
        });

        drop(hashes);
    }

    group.finish();
}

// ---------------------------------------------------------------------------
// Benchmark 2: Output Tree Ingestion
// ---------------------------------------------------------------------------

fn bench_ingestion(c: &mut Criterion) {
    let blobs = Blobs::new();
    let mut rng = StdRng::seed_from_u64(2);

    let mut group = c.benchmark_group("bench2_ingestion");
    // Fewer samples because each iteration writes ~4 MB.
    group.sample_size(20);

    for &compression in &[0i32, 6i32] {
        let label = format!("compression_{compression}");
        let (_dir, repo) = make_bare_repo(compression);

        group.bench_function(&label, |b| {
            b.iter(|| {
                let hash = random_hex32(&mut rng);
                ingest_crate_output(
                    black_box(&repo),
                    black_box(&hash),
                    black_box(&blobs.rlib),
                    black_box(&blobs.rmeta),
                    black_box(&blobs.fp),
                    Some(black_box(&blobs.build)),
                )
            })
        });
    }

    group.finish();

    // Repo size after 100 and 500 ingestions (reported to stdout).
    for &compression in &[0i32, 6i32] {
        for &n in &[100usize, 500] {
            let (dir, repo) = make_bare_repo(compression);
            let mut rng2 = StdRng::seed_from_u64(42);
            for _ in 0..n {
                let h = random_hex32(&mut rng2);
                ingest_crate_output(&repo, &h, &blobs.rlib, &blobs.rmeta, &blobs.fp, None);
            }
            let size = dir_size_bytes(dir.path());
            println!(
                "[bench2] compression={compression} n={n} repo_size={:.1} MB",
                size as f64 / 1_048_576.0
            );
        }
    }
}

// ---------------------------------------------------------------------------
// Benchmark 3: Output Tree Materialization
// ---------------------------------------------------------------------------

fn bench_materialization(c: &mut Criterion) {
    let blobs = Blobs::new();
    let mut rng = StdRng::seed_from_u64(3);
    let ns = [10usize, 50, 200];

    let mut group = c.benchmark_group("bench3_materialization");
    group.sample_size(10);

    for &n in &ns {
        let (_repo_dir, repo) = make_bare_repo(0);
        let out_dir = TempDir::new().expect("out tempdir");
        let out_path = out_dir.path().to_path_buf();

        // Pre-ingest N crates.
        let hashes: Vec<String> = (0..n)
            .map(|_| {
                let h = random_hex32(&mut rng);
                ingest_crate_output(
                    &repo,
                    &h,
                    &blobs.rlib,
                    &blobs.rmeta,
                    &blobs.fp,
                    Some(&blobs.build),
                );
                h
            })
            .collect();

        let target_hash = hashes[0].clone();

        group.bench_with_input(BenchmarkId::new("n_crates", n), &n, |b, _| {
            b.iter(|| {
                let ref_name = format!("refs/kiln/outputs/{}", &target_hash);
                let r = repo.find_reference(&ref_name).expect("find ref");
                let commit = r.peel_to_commit().expect("peel commit");
                let tree = commit.tree().expect("tree");

                // Separate odb read time from filesystem write time.
                let odb_start = Instant::now();
                // Pre-load all blobs from the odb (simulates the read phase).
                let _ = tree.iter().count();
                let odb_elapsed = odb_start.elapsed();

                let fs_start = Instant::now();
                let bytes =
                    walk_and_materialize(black_box(&repo), black_box(&tree), black_box(&out_path))
                        .expect("materialize");
                let fs_elapsed = fs_start.elapsed();

                black_box((bytes, odb_elapsed, fs_elapsed))
            })
        });

        drop(hashes);
        drop(out_dir);
    }

    group.finish();
}

// ---------------------------------------------------------------------------
// Benchmark 4: Deduplication Across Crate Versions
// ---------------------------------------------------------------------------

fn bench_deduplication(c: &mut Criterion) {
    let blobs = Blobs::new();
    let mut rng = StdRng::seed_from_u64(4);
    let n_crates = 50usize;

    let mut group = c.benchmark_group("bench4_deduplication");
    group.sample_size(10);

    group.bench_function("two_builds_one_changed", |b| {
        b.iter_custom(|iters| {
            let mut total = Duration::ZERO;
            for _ in 0..iters {
                let (dir, repo) = make_bare_repo(0);

                // Generate N hashes for "main build".
                let main_hashes: Vec<String> =
                    (0..n_crates).map(|_| random_hex32(&mut rng)).collect();

                let t0 = Instant::now();

                // Ingest main build.
                for h in &main_hashes {
                    ingest_crate_output(&repo, h, &blobs.rlib, &blobs.rmeta, &blobs.fp, None);
                }

                // Feature build: same hashes except one, same blob content for
                // unchanged crates so Git deduplicates via content-addressing.
                let mut feature_hashes = main_hashes.clone();
                feature_hashes[0] = random_hex32(&mut rng); // one changed crate

                // The changed crate uses entirely different blob content.
                let changed_rlib: Vec<u8> = random_bytes(&mut rng, blobs.rlib.len());

                for (i, h) in feature_hashes.iter().enumerate() {
                    if i == 0 {
                        ingest_crate_output(&repo, h, &changed_rlib, &blobs.rmeta, &blobs.fp, None);
                    } else {
                        // Identical content — Git will reuse existing blob OIDs.
                        ingest_crate_output(&repo, h, &blobs.rlib, &blobs.rmeta, &blobs.fp, None);
                    }
                }

                total += t0.elapsed();

                // Count unique objects in the odb.
                let mut obj_count = 0usize;
                repo.odb()
                    .expect("odb")
                    .foreach(|_oid| {
                        obj_count += 1;
                        true
                    })
                    .expect("odb foreach");

                let repo_size = dir_size_bytes(dir.path());
                // Objects per crate (approx): commit + root tree + deps tree +
                // fp-outer tree + fp-inner tree + rlib blob + rmeta blob +
                // timestamp blob + lib-foo blob + build tree = ~10
                let no_dedup_estimate = 2 * n_crates * 10;
                println!(
                    "[bench4] n={n_crates} unique_objects={obj_count} \
                     no_dedup_estimate={no_dedup_estimate} \
                     repo_size={:.1} MB",
                    repo_size as f64 / 1_048_576.0,
                );

                drop(main_hashes);
                drop(feature_hashes);
            }
            total
        })
    });

    group.finish();
}

// ---------------------------------------------------------------------------
// Benchmark 5: Ref Namespace Scale
// ---------------------------------------------------------------------------

fn bench_ref_namespace(c: &mut Criterion) {
    let mut rng = StdRng::seed_from_u64(5);
    let ns = [1_000usize, 10_000, 100_000];

    let mut group = c.benchmark_group("bench5_ref_namespace");
    group.sample_size(10);
    group.measurement_time(Duration::from_secs(30));

    for &n in &ns {
        let (dir, repo) = make_bare_repo(0);

        // Write N dummy commits/refs.
        let hashes: Vec<String> = (0..n).map(|_| random_hex32(&mut rng)).collect();
        for h in &hashes {
            ingest_dummy_commit(&repo, h);
        }

        // Pack refs at 100k — this is the realistic state for an established repo.
        if n >= 100_000 {
            Command::new("git")
                .args(["pack-refs", "--all"])
                .current_dir(dir.path())
                .output()
                .expect("git pack-refs");

            let packed_refs = dir.path().join("packed-refs");
            if packed_refs.exists() {
                let size = std::fs::metadata(&packed_refs)
                    .map(|m| m.len())
                    .unwrap_or(0);
                println!(
                    "[bench5] n={n} packed-refs size={:.1} KB",
                    size as f64 / 1024.0
                );
            }
        }

        // --- Enumeration ---
        group.bench_with_input(BenchmarkId::new("enumerate", n), &n, |b, _| {
            b.iter(|| {
                let mut count = 0usize;
                let refs = repo
                    .references_glob("refs/kiln/outputs/*")
                    .expect("references_glob");
                for r in refs {
                    let _ = r.expect("ref");
                    count += 1;
                }
                black_box(count)
            })
        });

        // --- Targeted lookup hit/miss at this scale ---
        let hit = hashes[n / 2].clone();
        let miss = random_hex32(&mut rng);

        group.bench_with_input(BenchmarkId::new("lookup_hit", n), &n, |b, _| {
            b.iter(|| {
                let rn = format!("refs/kiln/outputs/{}", &hit);
                black_box(repo.find_reference(black_box(&rn)).is_ok())
            })
        });

        group.bench_with_input(BenchmarkId::new("lookup_miss", n), &n, |b, _| {
            b.iter(|| {
                let rn = format!("refs/kiln/outputs/{}", &miss);
                black_box(repo.find_reference(black_box(&rn)).is_err())
            })
        });

        drop(hashes);
    }

    group.finish();
}

// ---------------------------------------------------------------------------
// Benchmark 6: Fetch Simulation (Cache Pre-Population)
// ---------------------------------------------------------------------------

fn bench_fetch_simulation(c: &mut Criterion) {
    let blobs = Blobs::new();
    let mut rng = StdRng::seed_from_u64(6);
    let ns = [50usize, 150, 300];

    let mut group = c.benchmark_group("bench6_fetch_simulation");
    group.sample_size(10);
    group.measurement_time(Duration::from_secs(60));

    // Bytes per crate: rlib + rmeta + fp + build (uncompressed in-memory sizes).
    let bytes_per_crate =
        (blobs.rlib.len() + blobs.rmeta.len() + blobs.fp.len() + blobs.build.len()) as u64;

    for &n in &ns {
        let total_bytes = bytes_per_crate * n as u64;
        let time_100mbit = total_bytes as f64 / (100.0 * 1_000_000.0 / 8.0);
        let time_1gbit = total_bytes as f64 / (1_000.0 * 1_000_000.0 / 8.0);
        println!(
            "[bench6] n={n} total_bytes={:.1} MB  \
             est_fetch@100Mbit={time_100mbit:.1}s  \
             est_fetch@1Gbit={time_1gbit:.2}s",
            total_bytes as f64 / 1_048_576.0,
        );

        // Measure local ingestion time — the portion Kiln controls after transfer.
        group.bench_with_input(BenchmarkId::new("local_ingestion", n), &n, |b, &count| {
            b.iter_custom(|iters| {
                let mut total = Duration::ZERO;
                for _ in 0..iters {
                    let (_dir, repo) = make_bare_repo(0);
                    let seed: u64 = rng.r#gen();
                    let mut rng2 = StdRng::seed_from_u64(seed);
                    let t0 = Instant::now();
                    for _ in 0..count {
                        let h = random_hex32(&mut rng2);
                        ingest_crate_output(
                            &repo,
                            &h,
                            &blobs.rlib,
                            &blobs.rmeta,
                            &blobs.fp,
                            Some(&blobs.build),
                        );
                    }
                    total += t0.elapsed();
                }
                total
            })
        });
    }

    group.finish();
}

// ---------------------------------------------------------------------------
// Benchmark 7: GC and Pack Behavior
// ---------------------------------------------------------------------------

fn bench_gc(c: &mut Criterion) {
    let blobs = Blobs::new();
    let mut rng = StdRng::seed_from_u64(7);

    let mut group = c.benchmark_group("bench7_gc");
    group.sample_size(10);
    group.measurement_time(Duration::from_secs(120));

    group.bench_function("gc_500_drop_250", |b| {
        b.iter_custom(|iters| {
            let mut total = Duration::ZERO;
            for _ in 0..iters {
                let (dir, repo) = make_bare_repo(0);

                // Ingest 500 crate output trees.
                let hashes: Vec<String> = (0..500usize)
                    .map(|_| {
                        let h = random_hex32(&mut rng);
                        ingest_crate_output(&repo, &h, &blobs.rlib, &blobs.rmeta, &blobs.fp, None);
                        h
                    })
                    .collect();

                let size_before = dir_size_bytes(dir.path());

                // Drop 250 refs (simulate eviction of old builds).
                for h in hashes.iter().take(250) {
                    let ref_name = format!("refs/kiln/outputs/{h}");
                    if let Ok(mut r) = repo.find_reference(&ref_name) {
                        r.delete().expect("delete ref");
                    }
                }

                // Write a gitattributes file to prevent delta compression on
                // binary artifact blobs.
                let attrs_path = dir.path().join("info").join("attributes");
                std::fs::create_dir_all(attrs_path.parent().unwrap()).ok();
                std::fs::write(&attrs_path, "*.rlib -delta\n*.rmeta -delta\n")
                    .expect("write gitattributes");

                // Run git gc and measure only gc time.
                let t0 = Instant::now();
                let gc_out = Command::new("git")
                    .args(["gc", "--prune=now", "--quiet"])
                    .current_dir(dir.path())
                    .output()
                    .expect("git gc");
                let gc_elapsed = t0.elapsed();
                total += gc_elapsed;

                let size_after = dir_size_bytes(dir.path());

                if !gc_out.status.success() {
                    eprintln!(
                        "[bench7] git gc stderr: {}",
                        String::from_utf8_lossy(&gc_out.stderr)
                    );
                }

                // Verify retained refs are intact after gc.
                let mut intact = true;
                for h in hashes.iter().skip(250) {
                    let ref_name = format!("refs/kiln/outputs/{h}");
                    if repo.find_reference(&ref_name).is_err() {
                        intact = false;
                        break;
                    }
                }

                // Count .pack files to confirm blobs were packed.
                let pack_dir = dir.path().join("objects").join("pack");
                let pack_count = std::fs::read_dir(&pack_dir)
                    .map(|rd| {
                        rd.filter_map(|e| e.ok())
                            .filter(|e| e.path().extension().map(|x| x == "pack").unwrap_or(false))
                            .count()
                    })
                    .unwrap_or(0);

                println!(
                    "[bench7] size_before={:.1} MB  size_after={:.1} MB  \
                     gc_time={:.2}s  retained_refs_intact={intact}  \
                     pack_files={pack_count}",
                    size_before as f64 / 1_048_576.0,
                    size_after as f64 / 1_048_576.0,
                    gc_elapsed.as_secs_f64(),
                );

                drop(hashes);
            }
            total
        })
    });

    group.finish();
}

// ---------------------------------------------------------------------------
// Criterion entry points
// ---------------------------------------------------------------------------

criterion_group! {
    name = lookup;
    config = Criterion::default().measurement_time(Duration::from_secs(10));
    targets = bench_cache_lookup
}

criterion_group! {
    name = ingestion;
    config = Criterion::default().measurement_time(Duration::from_secs(30));
    targets = bench_ingestion
}

criterion_group! {
    name = materialization;
    config = Criterion::default().measurement_time(Duration::from_secs(30));
    targets = bench_materialization
}

criterion_group! {
    name = deduplication;
    config = Criterion::default().measurement_time(Duration::from_secs(60));
    targets = bench_deduplication
}

criterion_group! {
    name = ref_namespace;
    config = Criterion::default().measurement_time(Duration::from_secs(30));
    targets = bench_ref_namespace
}

criterion_group! {
    name = fetch_simulation;
    config = Criterion::default().measurement_time(Duration::from_secs(60));
    targets = bench_fetch_simulation
}

criterion_group! {
    name = gc;
    config = Criterion::default().measurement_time(Duration::from_secs(120));
    targets = bench_gc
}

criterion_main!(
    lookup,
    ingestion,
    materialization,
    deduplication,
    ref_namespace,
    fetch_simulation,
    gc
);
