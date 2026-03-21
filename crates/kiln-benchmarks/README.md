# kiln-benchmarks

Benchmarks measuring Git's object store performance under Kiln's build cache
access patterns. These benchmarks answer the critical questions:

- How fast is cache lookup by action hash?
- How fast is output tree ingestion for compiled artifacts?
- How does performance scale as the ref namespace grows?
- Does Git's content-addressing actually deduplicate unchanged crate outputs?
- What is the realistic cost of a "fresh clone, no build needed" scenario?

## Running the Benchmarks

```sh
cargo bench --package kiln-benchmarks
```

Results are written to `target/criterion/`. Open
`target/criterion/report/index.html` for the full HTML report.

To run a single benchmark group:

```sh
# Benchmark 1: cache lookup
cargo bench --package kiln-benchmarks --bench kiln -- bench1_cache_lookup

# Benchmark 2: output tree ingestion
cargo bench --package kiln-benchmarks --bench kiln -- bench2_ingestion

# Benchmark 3: output tree materialization
cargo bench --package kiln-benchmarks --bench kiln -- bench3_materialization

# Benchmark 4: deduplication
cargo bench --package kiln-benchmarks --bench kiln -- bench4_deduplication

# Benchmark 5: ref namespace scale
cargo bench --package kiln-benchmarks --bench kiln -- bench5_ref_namespace

# Benchmark 6: fetch simulation
cargo bench --package kiln-benchmarks --bench kiln -- bench6_fetch_simulation

# Benchmark 7: GC and pack behavior
cargo bench --package kiln-benchmarks --bench kiln -- bench7_gc
```

> **Note:** Benchmarks 5, 6, and 7 are slow by design — they exercise 10k–100k
> refs and 500-crate ingestion at realistic scale. Allow 5–30 minutes for a
> full run.

## Benchmark Descriptions

### Benchmark 1 — Action Hash Cache Lookup

Tests the core cache check: "does a cached output exist for this action hash?"

Pre-populates a bare repo with N refs under `refs/kiln/outputs/<hash>` at
N = 100, 1,000, and 10,000. Each ref points to a commit whose tree contains
realistic build output blobs (a 4 MB `.rlib`, 100 KB `.rmeta`, and fingerprint
files). Measures both the **hit case** (ref exists, peel to tree OID) and the
**miss case** (ref absent).

**Scale target:** hit < 5 ms, miss < 2 ms at N = 10,000.

### Benchmark 2 — Output Tree Ingestion

Tests writing a compiled crate's outputs into the object store.

Writes realistic blobs (4 MB `.rlib`, 100 KB `.rmeta`, 1 KB fingerprint, 50 KB
build output) and constructs the full tree/commit/ref chain. Compares
`core.looseCompression = 0` (Kiln's recommended setting for binary artifact
repos) against the default compression level 6. Also reports total repo size
after 100 and 500 ingestions.

**Scale target:** < 500 ms/crate for a 4 MB rlib + metadata.

### Benchmark 3 — Output Tree Materialization

Tests restoring cached outputs to the working directory — the operation that
replaces compilation on a cache hit.

Pre-ingests N crate output trees (N = 10, 50, 200) and measures reading each
blob from the odb and writing it to the filesystem. Reports odb read time
separately from filesystem write time to identify the bottleneck.

**Scale target:** < 200 ms/crate from local odb; < 30 s total for 300 crates.

### Benchmark 4 — Deduplication Across Crate Versions

Tests Git's structural deduplication: two builds that share most crates should
not duplicate those crates' blobs in the object store.

Ingests a "main branch build" (N crates) and a "feature branch build" (same N
crates, one changed). Unchanged crates use identical blob content so Git's
content-addressing deduplicates them. Reports the unique object count and
compares it to the theoretical no-dedup count.

**Scale target:** > 95% deduplication when 1 crate changes out of N.

### Benchmark 5 — Ref Namespace Scale

Tests whether Git's ref storage degrades as `refs/kiln/outputs/` grows to
represent months of CI builds.

Writes refs in batches and measures enumeration time at 1,000, 10,000, and
100,000 refs. Runs `git pack-refs --all` before the 100,000-ref measurement
(the realistic state for an established repo). Also measures targeted lookup
at each scale to confirm O(1) lookup is preserved.

**Scale target:** enumeration < 500 ms at 10,000 packed refs.

### Benchmark 6 — Fetch Simulation (Cache Pre-Population)

Tests how long a fresh clone takes to receive cached outputs for a full project
build — the "fresh clone, no build needed" demo scenario.

Since libgit2 in-process fetch does not reflect real network conditions, reports
two things: the total bytes that would need to be transferred for N crates
(N = 50, 150, 300), and the local ingestion time after a hypothetical transfer.
Also prints estimated fetch time at 100 Mbit/s and 1 Gbit/s for comparison with
alternatives like sccache or Artifactory.

### Benchmark 7 — GC and Pack Behavior

Tests whether Git's garbage collection handles a Kiln object store gracefully.

Ingests 500 crate output trees, drops 250 refs (simulating eviction of old
builds), then runs `git gc --prune=now`. Reports repo size before and after GC,
GC duration, whether retained refs remain intact, and pack file count. Also
writes a `gitattributes` file marking `.rlib` and `.rmeta` with `-delta` to
prevent delta compression on binary artifact blobs.

## Configuration Notes

All benchmark repos are initialized with `core.looseCompression = 0` (except
Benchmark 2's compression comparison). This reflects the tuning recommended for
repos that store large binary artifact blobs, where zlib compression overhead
exceeds any size benefit on already-compressed or random binary data.

All repos are created fresh in temporary directories with no remotes configured.
Transport/remote benchmarks are out of scope and will be covered separately.

## Scale Targets Summary

| Benchmark | Operation | Target | At Scale |
|-----------|-----------|--------|----------|
| 1 | Cache lookup — hit | < 5 ms | 10,000 refs |
| 1 | Cache lookup — miss | < 2 ms | 10,000 refs |
| 2 | Output ingestion | < 500 ms/crate | 4 MB rlib + metadata |
| 3 | Materialization | < 200 ms/crate | from local odb |
| 3 | Full materialization | < 30 s | 300 crates (Zed scale) |
| 5 | Ref enumeration | < 500 ms | 10,000 refs packed |
| 4 | Deduplication | > 95% savings | 1 crate changed of N |

## Results

> Results below are populated after running `cargo bench --package kiln-benchmarks`
> on the target machine. Until then, this section serves as a template.

### Environment

- **Date:** _not yet run_
- **Machine:** _not yet run_
- **OS:** _not yet run_
- **Rust:** _not yet run_
- **libgit2:** _not yet run_

### Benchmark 1 — Cache Lookup Results

| N refs | Hit latency (mean) | Miss latency (mean) | Hit target met? | Miss target met? |
|-------:|--------------------|---------------------|-----------------|------------------|
| 100 | — | — | — | — |
| 1,000 | — | — | — | — |
| 10,000 | — | — | — | — |

### Benchmark 2 — Ingestion Results

| Compression | Mean latency | Throughput (crates/s) |
|-------------|-------------|----------------------|
| Level 0 | — | — |
| Level 6 | — | — |

| N crates | Compression | Repo size |
|---------:|-------------|-----------|
| 100 | Level 0 | — |
| 100 | Level 6 | — |
| 500 | Level 0 | — |
| 500 | Level 6 | — |

### Benchmark 3 — Materialization Results

| N crates | Mean latency/crate | Total bytes written | ODB time | FS write time |
|---------:|--------------------|---------------------|----------|---------------|
| 10 | — | — | — | — |
| 50 | — | — | — | — |
| 200 | — | — | — | — |

**Bottleneck:** _not yet determined_

### Benchmark 4 — Deduplication Results

| N crates | Unique objects | No-dedup estimate | Dedup ratio | Repo size |
|---------:|---------------|-------------------|-------------|-----------|
| 50 | — | — | — | — |

**Finding:** _not yet run_

### Benchmark 5 — Ref Namespace Scale Results

| N refs | Enumeration latency | Lookup hit | Lookup miss | Packed-refs size |
|-------:|--------------------:|------------|-------------|------------------|
| 1,000 | — | — | — | — |
| 10,000 | — | — | — | — |
| 100,000 (packed) | — | — | — | — |

**Enumeration becomes unacceptably slow (> 1 s) at:** _not yet determined_

### Benchmark 6 — Fetch Simulation Results

| N crates | Total bytes | Est. fetch @ 100 Mbit/s | Est. fetch @ 1 Gbit/s | Local ingestion time |
|---------:|-------------|-------------------------|-----------------------|----------------------|
| 50 | — | — | — | — |
| 150 | — | — | — | — |
| 300 | — | — | — | — |

### Benchmark 7 — GC Results

| Metric | Value |
|--------|-------|
| Repo size before GC | — |
| Repo size after GC | — |
| GC duration | — |
| Retained refs intact | — |
| Pack files created | — |
| Binary blobs delta-compressed | — |

## Findings and Recommendations

_This section will be filled in after results are collected._

Key questions to answer:

1. **Does cache lookup degrade with ref count?** If miss latency grows beyond
   2 ms at 10,000 refs, consider a derived index (e.g. a SQLite sidecar
   mapping action hashes to object IDs) to bypass Git's ref lookup entirely.

2. **Does `core.looseCompression = 0` meaningfully speed up ingestion?** If
   compression level 6 and level 0 show similar latency, the recommendation in
   the Kiln spec may not be necessary on modern hardware.

3. **Is deduplication actually happening?** If blob content is stable across
   unchanged crates (no embedded timestamps, no ASLR-derived addresses in debug
   info), the dedup ratio should approach (N−1)/N. If it is not, action hash
   computation must normalize those inputs before hashing.

4. **Is the 30-second full materialization target achievable?** At 200 ms/crate,
   300 crates = 60 s — twice the target. If this is the case, the recommended
   mitigation is parallel materialization (spawn one thread per crate) or
   switching from full tree materialization to hardlinking from a local object
   cache directory.

5. **Does GC correctly prune dropped refs?** If unreachable objects are retained
   after `git gc --prune=now`, the eviction strategy must be revisited.
