# git-kiln-bench: Proving Git as a Build CAS

## Goal

Answer one question with numbers: can Git's object store serve as the content-addressed cache for a build system at realistic scale?

Every benchmark compares against the baseline: tar+zstd to local disk (proxy for S3/sccache).
Git doesn't need to be faster.
It needs to be close enough that the deduplication, provenance, and transport advantages justify the overhead.

## Crate Structure

```text
git-kiln-bench/
├── Cargo.toml
├── src/
│   ├── lib.rs              # shared types, repo setup, artifact generation
│   ├── gen.rs              # realistic artifact generators
│   ├── bin/
│   │   ├── ingest.rs       # Benchmark 1: blob write throughput
│   │   ├── tree_build.rs   # Benchmark 2: tree construction
│   │   ├── cache_index.rs  # Benchmark 3: cache index read/write
│   │   ├── fetch.rs        # Benchmark 4: object fetch from remote
│   │   ├── soak.rs         # Benchmark 5: week-long CI simulation
│   │   └── baseline.rs     # Benchmark 6: tar+zstd comparison
```

Dependencies: `git2` (libgit2 bindings), `criterion` for microbenchmarks, `tempfile`, `rand`, `zstd` for baseline comparison.

## Artifact Generation (gen.rs)

Generate fake but realistic build outputs.
The byte distribution matters — random data won't compress like real artifacts, but zeros won't either.

Approach: read a few real `.rlib` and `.so` files, compute byte frequency distributions, generate synthetic artifacts matching those distributions at configurable sizes.

Realistic target profile (medium Rust workspace, ~30 crates):

| Artifact type | Count | Size range | Total |
|---|---|---|---|
| .rlib | 120 | 50KB–5MB | ~150MB |
| .rmeta | 120 | 10KB–500KB | ~25MB |
| .so/.dylib | 8 | 1MB–50MB | ~100MB |
| .d files | 120 | 1KB–10KB | ~1MB |
| build script outputs | 15 | 1KB–1MB | ~5MB |

Total per build: ~280MB, ~380 files.

For the C++/ROS case, scale up: 200 packages, 500MB–2GB total.

## Benchmark 1: Ingestion Throughput

**Question:** How fast can we write N blobs into the ODB and construct a tree?

```rust
// Pseudocode
fn bench_ingest(compression_level: i32, artifacts: &[Artifact]) -> IngestResult {
    let repo = init_bare_repo();
    repo.config().set_i32("core.looseCompression", compression_level);

    let start = Instant::now();
    let mut tree = TreeBuilder::new(&repo);
    for artifact in artifacts {
        let oid = repo.blob(&artifact.bytes);  // git_odb_write
        tree.insert(&artifact.name, oid, artifact.filemode);
    }
    let tree_oid = tree.write();
    let commit_oid = repo.commit(tree_oid, ...);

    IngestResult {
        wall_time: start.elapsed(),
        throughput_mb_s: total_bytes / elapsed_secs,
        repo_size: dir_size(repo.path()),
    }
}
```

Parameter matrix:

- Compression level: 0, 1, 6 (default)
- Artifact set: small (30 crates), medium (100 crates), large (300 crates)
- With/without `-delta` gitattributes on binaries

Key metrics: MB/s write throughput, wall-clock time, repo size.

## Benchmark 2: Tree Construction at Scale

**Question:** How expensive is building nested trees for a large output?

Build actions produce output trees with structure:

```text
deps/
  libfoo.rlib
  libfoo.rmeta
  libbar.rlib
  ...
.fingerprint/
  foo-<hash>/
    ...
build/
  foo-<hash>/
    out/
      generated.rs
```

Measure tree construction separately from blob writes.
This isolates the cost of `git_treebuilder` operations and nested tree assembly.

## Benchmark 3: Cache Index Operations

**Question:** How fast is action-hash → commit-OID lookup through a tree-based index?

The cache index is a tree under `refs/kiln/cache` with two-level fanout:

```text
ab/
  cd1234.../
    commit-oid    # blob containing the OID
  ef5678.../
    commit-oid
```

Benchmark:

- Write 10K, 50K, 200K entries (simulating months of CI)
- Random lookup latency (p50, p95, p99)
- Sequential scan time
- Index update: add one entry to an existing 50K-entry index (requires tree rewrite)

The update cost is the concern.
Adding one entry means rewriting the fanout subtree and the root tree.
Compare to: SQLite lookup, flat file scan.

## Benchmark 4: Fetch Simulation

**Question:** How fast can a client restore a full build's outputs from a remote?

Setup:

- Local bare repo as "remote" (same machine, eliminates network variable)
- Client repo with partial clone (`--filter=blob:none`)
- Remote has 50 build commits with realistic output trees

Measure:

- Fetch one output tree by commit OID (all blobs materialized)
- Fetch 30 output trees (full build restore, 30-package project)
- Incremental fetch: remote has new build, 28/30 packages unchanged

Compare to: `curl` a tarball of equivalent size from localhost.

For the network case (separate benchmark or flag): fetch over localhost TCP to measure protocol overhead without real network latency.

## Benchmark 5: Soak Test

**Question:** What happens to repo size and GC time over simulated weeks of CI?

```rust
fn soak(config: SoakConfig) -> SoakResult {
    // config: builds_per_day, days, packages_per_build, ttl_days
    let repo = init_bare_repo();
    let mut daily_stats = vec![];

    for day in 0..config.days {
        // Write builds
        for _ in 0..config.builds_per_day {
            let artifacts = generate_build(config.packages_per_build);
            ingest(&repo, &artifacts, day);
        }

        // Expire refs older than TTL
        expire_refs(&repo, day - config.ttl_days);

        // GC
        let gc_start = Instant::now();
        repo.gc();  // or shell out to `git gc`
        let gc_time = gc_start.elapsed();

        daily_stats.push(DayStats {
            day,
            repo_size: dir_size(repo.path()),
            gc_time,
            loose_objects: count_loose(&repo),
            pack_size: pack_size(&repo),
        });
    }
}
```

Realistic parameters:

- 20 builds/day (CI on push for active team)
- 30 days simulated
- 30 packages per build, ~280MB artifacts
- 7-day TTL on output refs

Output: time-series of repo size, GC duration, pack file count/size.

## Benchmark 6: Baseline Comparison

**Question:** How does Git compare to the dumb approach?

For each operation, measure the equivalent non-Git path:

| Git operation | Baseline equivalent |
|---|---|
| Blob ingest at level 0 | Write raw files to disk |
| Blob ingest at level 6 | zstd compress + write |
| Tree construction + commit | tar + write |
| Cache index lookup | SQLite key-value lookup |
| Full restore from remote | Download tarball |
| Incremental restore | rsync |

Same artifact sets, same machine.
Produce a comparison table.

## Development Sequence

### Week 1: Scaffolding + Ingestion

1. Set up crate, `git2` dependency, tempdir-based repo creation
2. Implement artifact generator with realistic byte distributions
3. Benchmark 1: ingestion at compression levels 0/1/6
4. Benchmark 6 (partial): baseline raw write and zstd write

Deliverable: first numbers on MB/s ingestion.
This is the most likely failure point — if ingestion is catastrophically slow, stop here.

### Week 2: Trees + Cache Index

5. Benchmark 2: tree construction at varying depths/widths
6. Benchmark 3: cache index CRUD at 10K/50K/200K entries
7. Investigate whether tree-based index is viable or if a sidecar (SQLite, flat file) is needed for the index specifically

Deliverable: cache lookup latency numbers.
If index update is too expensive at 50K entries, the index design needs revision before proceeding.

### Week 3: Fetch + Restore

8. Benchmark 4: local fetch simulation with partial clone
9. Benchmark 4 (extended): localhost TCP fetch
10. Benchmark 6 (continued): tarball download comparison

Deliverable: restore latency numbers.
This is the second most likely failure point — if fetching a build's outputs takes 30s vs 2s for a tarball, the developer experience suffers.

### Week 4: Soak + Report

11. Benchmark 5: 30-day soak test
12. Full baseline comparison table
13. Write-up with charts: throughput, latency, size over time, GC cost

Deliverable: a single document with every number, every comparison, every chart.
This is what you publish.

## Success Criteria

Git-as-CAS is viable if:

- **Ingestion**: >100 MB/s at compression level 0 (a 280MB build stored in <3s)
- **Cache lookup**: <10ms p99 at 50K entries
- **Full restore**: <5s for 280MB output tree from local remote (same order as tarball)
- **Soak stability**: repo size reaches steady state under TTL, doesn't grow unboundedly
- **GC**: <30s on a week's worth of builds (can run in background)

If any of these fail, the benchmark will have identified exactly where, which tells you whether the design needs revision or Git needs replacement.

## What This Doesn't Test

- Real network latency (test locally first, add network round-trip later)
- Concurrent writers (important but separate — test after single-writer is proven)
- libgit2 vs git CLI performance (benchmark both if ingestion is borderline)
- Packfile negotiation efficiency for partial clone (protocol-level concern, hard to benchmark in isolation)

These are follow-ups.
The core question — is Git fast enough as a local and local-remote CAS — comes first.
