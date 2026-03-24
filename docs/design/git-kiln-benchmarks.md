# Kiln Benchmark Instructions

## Goal

Measure whether Git's object store can handle Kiln's build cache access patterns at realistic scale.
The critical questions are: how fast is cache lookup by action hash, how fast is output tree ingestion for compiled artifacts, and how does fetch performance scale when pre-populating a fresh clone.

## Setup

Create a new Rust project:

```text
cargo new --lib kiln-benchmarks
cd kiln-benchmarks
```

Add dependencies to `Cargo.toml`:

```toml
[dev-dependencies]
criterion = { version = "0.5", features = ["html_reports"] }
git2 = "0.19"
tempfile = "3"
rand = "0.8"
sha2 = "0.10"
```

Add benchmark harness config:

```toml
[[bench]]
name = "kiln"
harness = false
```

Create `benches/kiln.rs`.

All benchmarks use temporary repositories created via `tempfile::TempDir` and `git2::Repository::init_bare`.
Use `git2::Repository::open` for the client-side repo in fetch benchmarks.

One important configuration: set `core.looseCompression` to 0 for repos that store binary artifact blobs.
Do this via `repo.config()?.set_i32("core.looseCompression", 0)`.
This reflects the tuning described in the Kiln spec and must be in place before ingesting binary blobs or the benchmarks will not reflect real performance.

## Benchmark 1: Action Hash Cache Lookup

**What it tests:** The core cache check — "does a cached output exist for this action hash?"

**Setup:** Pre-populate a repo with N refs under `refs/kiln/outputs/<hash>` (N = 100, 1000, 10000).
Each ref points to a commit whose tree contains a realistic set of build output blobs (see blob sizes below).
Use random 32-character hex strings as action hashes.

Realistic output tree structure per action (simulating one compiled Rust crate):

```text
<action-hash>/
  deps/
    libfoo.rlib       # 2-8 MB binary
    libfoo.rmeta      # 50-200 KB binary
  .fingerprint/
    foo-<hash>/
      invoked.timestamp   # tiny
      lib-foo             # ~1 KB
  build/              # may be empty for most crates
```

**Measured operation (hit case):**

1. Given an action hash string, check whether
   `refs/kiln/outputs/<hash>` exists using `repo.find_reference(...)`
2. If it exists, read the tip commit and return the tree OID

**Measured operation (miss case):**

1. Same lookup on a hash not present in the repo

**Report:** Latency (ms) for hit and miss at each N.
The miss case must be fast — it runs for every action on a cold cache.
The question is whether ref lookup degrades as the ref namespace grows.

## Benchmark 2: Output Tree Ingestion

**What it tests:** Writing a compiled crate's outputs into the object store.

**Setup:** Generate synthetic binary blobs of realistic sizes in memory using random bytes (do not use `rand::fill` on a huge buffer — generate once and reuse):

- One `.rlib` blob: 4 MB
- One `.rmeta` blob: 100 KB
- One fingerprint blob: 1 KB
- One build output blob (optional): 50 KB

**Measured operation:**

1. Write each blob to the odb using `repo.blob(data)`
2. Construct a tree with the correct structure using `repo.treebuilder`
3. Write a commit pointing at that tree
4. Update `refs/kiln/outputs/<action-hash>` to point at the commit

Also measure with `core.looseCompression = 0` versus the default (level 6).
This is the key comparison the Kiln spec claims matters for binary artifact ingestion speed.

**Report:** Latency (ms) per ingestion at compression level 0 and level 6.
Throughput (crates/sec).
Total repo size after ingesting 100, 500 crates.

## Benchmark 3: Output Tree Materialization

**What it tests:** Restoring cached outputs to the working directory — the operation that replaces compilation on a cache hit.

**Setup:** Pre-ingest N crate output trees (N = 10, 50, 200, representing a subset of Zed's ~300+ crates).
Use realistic blob sizes from Benchmark 2.

**Measured operation:**

1. Given an action hash, fetch the output tree from the odb
2. Walk the tree using `repo.find_tree(oid)?.iter()`
3. Write each blob to a target directory using standard filesystem writes
   (simulate materializing to `target/debug/deps/`)

**Report:** Latency (ms) per materialization at each N.
Total bytes written.
This should be dominated by filesystem write speed, not Git overhead — confirm this by comparing odb read time versus total materialization time.

## Benchmark 4: Deduplication Across Crate Versions

**What it tests:** Git's structural deduplication — two builds that share most crates should not duplicate those crates' blobs in the object store.

**Setup:** Ingest two sets of crate outputs representing "main branch build" and "feature branch build."
The feature branch changes one crate; all others are identical.
Use identical blob content for unchanged crates so Git's content-addressing deduplicates them.

**Measured operation:**

1. Ingest the main branch build (N crates)
2. Ingest the feature branch build (N crates, 1 changed)
3. Count unique objects in the odb using `repo.odb()?.foreach()`
4. Compare total object count to 2*N (no dedup) versus N+delta (full dedup)

**Report:** Object count and total repo size after both ingestions.
The expected result is that only the changed crate's blobs are duplicated — confirm this.
If deduplication is not happening (because blob content differs by metadata, timestamps, etc.), report that finding explicitly — it means action hash computation needs to normalize those inputs.

## Benchmark 5: Ref Namespace Scale

**What it tests:** Whether Git's ref storage degrades as `refs/kiln/outputs/` grows to represent months of CI builds.

**Setup:** Write refs in batches, measuring ref enumeration time at each checkpoint:

- 1,000 refs (small project, few weeks of CI)
- 10,000 refs (medium project, months of CI)
- 100,000 refs (large project, years of CI)

Each ref is a lightweight ref pointing at a commit (no need for full output trees — just dummy commits).

**Measured operation:**

1. Enumerate all refs under `refs/kiln/outputs/` using
   `repo.references_glob("refs/kiln/outputs/*")`
2. Count them (do not read commits — just enumerate ref names)

Also measure targeted lookup (Benchmark 1) at 100,000 refs to confirm it does not degrade with namespace size.

**Report:** Enumeration latency (ms) at each checkpoint.
Lookup latency at 100,000 refs.
Packed-refs file size.
At what N does enumeration become unacceptably slow (define unacceptable as > 1 second).

Note: run `git pack-refs --all` via `std::process::Command` before the 100,000 ref benchmark — loose refs at that scale will be artificially slow and packed refs is the realistic state for an old repo.

## Benchmark 6: Fetch Simulation (Cache Pre-Population)

**What it tests:** How long it takes a fresh clone to fetch cached outputs for a full project build — the "fresh clone, no build needed" demo scenario.

**Setup:** Create a "remote" bare repo pre-populated with output trees for N crates (N = 50, 150, 300 — representing a subset, half, and all of Zed's crates).
Create a "local" empty repo.
Use `git2::Remote` to simulate fetch.

Since libgit2's in-process fetch won't reflect real network conditions, measure two things separately:

1. Object transfer: how many bytes need to be fetched (sum of all blob sizes
   for N crates)
2. Local ingestion: how long it takes to write N crate output trees to the
   local odb after transfer (use Benchmark 2's write path, batched)

**Report:** Total bytes transferred at each N.
Local ingestion time.
Estimated fetch time at 100 Mbit/s and 1 Gbit/s connections (bytes / bandwidth).
The question is whether the total data size is competitive with alternatives like sccache or Artifactory for the same set of artifacts.

## Benchmark 7: gc and Pack Behavior

**What it tests:** Whether Git's garbage collection handles a Kiln object store gracefully — specifically, that dropping old build refs frees space correctly and that pack file behavior is reasonable.

**Setup:** Ingest 500 crate output trees.
Then drop 250 of the refs (simulating eviction of old builds).
Run `git gc` via `std::process::Command` (libgit2 does not expose gc directly).

**Measured operation:**

1. Repo size before gc
2. Run gc
3. Repo size after gc
4. Verify dropped refs' objects are pruned (unreachable objects gone)
5. Verify retained refs' objects are intact

Also measure pack file size versus loose object size for binary blobs with `core.looseCompression = 0`.
Confirm that gc does not attempt delta compression on `.rlib` blobs when `.gitattributes -delta` is set.

**Report:** Repo size before and after gc.
Time taken by gc.
Whether binary blobs are correctly excluded from delta compression.

## Scale Targets

| Operation | Target | At scale |
|-----------|--------|----------|
| Cache lookup (hit) | < 5ms | 10,000 cached actions |
| Cache lookup (miss) | < 2ms | 10,000 cached actions |
| Output ingestion | < 500ms/crate | 4MB rlib + metadata |
| Materialization | < 200ms/crate | from local odb |
| Full materialization | < 30s | 300 crates (Zed) |
| Ref enumeration | < 500ms | 10,000 refs packed |
| Deduplication | > 95% | 1 crate changed of 300 |

The full materialization target is the demo-critical number. 30 seconds for a "fresh clone, no build needed" scenario is the upper bound for the demo to be impressive.
If it exceeds this, report the actual number and identify whether the bottleneck is odb reads, filesystem writes, or ref lookup.

## Reporting

Use Criterion's default HTML report output.
For each benchmark report:

- Mean latency and standard deviation
- The N at which latency first exceeds the target
- Whether the bottleneck is odb, filesystem, or ref operations
- Compression level 0 vs default where applicable

Save results to `target/criterion/`.
Include a `RESULTS.md` summarizing findings, any targets missed, and recommended mitigations if targets are missed (e.g. derived index, partial clone, explicit fetch instead of full materialization).
