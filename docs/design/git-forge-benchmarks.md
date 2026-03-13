# Forge Benchmark Instructions

## Goal

Measure whether Git's object store and ref model can handle Forge's data access
patterns at realistic scale. These benchmarks are not micro-benchmarks of
libgit2 internals — they test the specific operations Forge performs at the
scale a large active project would produce.

## Setup

Create a new Rust project:

```
cargo new --lib forge-benchmarks
cd forge-benchmarks
```

Add dependencies to `Cargo.toml`:

```toml
[dev-dependencies]
criterion = { version = "0.5", features = ["html_reports"] }
git2 = "0.19"
tempfile = "3"
rand = "0.8"
```

Add benchmark harness config:

```toml
[[bench]]
name = "forge"
harness = false
```

Create `benches/forge.rs`.

All benchmarks use a temporary bare repository created fresh per benchmark
group. Use `tempfile::TempDir` for cleanup. Initialize with `git2::Repository::init_bare`.

## Benchmark 1: Issue Creation at Scale

**What it tests:** The counter ref CAS protocol and ref-per-issue write pattern.

**Setup:** Initialize a repo. Pre-create N issues (N = 100, 1000, 10000) by
writing commits to `refs/meta/issues/<id>`. Each issue commit has a tree with
two blobs: `meta` (TOML, ~200 bytes) and `body` (Markdown, ~500 bytes).

**Measured operation:** Create one new issue using the optimistic CAS protocol:
1. Read `refs/meta/counters`, parse current value N
2. Write a new commit to `refs/meta/issues/<N+1>` with a tree containing `meta`
   and `body` blobs
3. Atomically update `refs/meta/counters` to N+1 using
   `git2::Reference::set_target` with the expected OID as the compare value

**Report:** Throughput (issues/sec) and latency (ms/issue) at each N. Verify
that CAS contention (simulated by two threads racing) produces correct retry
behavior without data loss.

## Benchmark 2: Issue Listing

**What it tests:** Ref enumeration for "list all open issues."

**Setup:** Pre-create N issues (N = 100, 1000, 10000). Half are open, half
closed (state field in `meta` blob).

**Measured operation:** 
1. Enumerate all refs matching `refs/meta/issues/*` using
   `repo.references_glob("refs/meta/issues/*")`
2. For each ref, read the tip commit's tree, read the `meta` blob, parse state
3. Return the list of open issue IDs

**Report:** Latency (ms) and memory usage at each N. This is the hot path for
issue list views. The question is whether ref glob + blob read is fast enough
without a derived index.

## Benchmark 3: Blob-Anchored Comment Lookup

**What it tests:** The core Forge comment query — "show all comments on this
file."

**Setup:** Using `git2`'s notes API (or direct tree manipulation on a
`refs/metadata/comments` ref), write M comments across K distinct blob OIDs
(simulate K files, each with M/K comments). Use realistic values:
K=50 files, M=500 comments total.

Structure each comment entry as a subtree under the blob OID:

```
refs/metadata/comments → commit → tree
  <blob-oid>/
    <comment-id>/
      meta    # TOML blob
      body    # Markdown blob
```

**Measured operation:**
1. Given a current file, compute its blob OID via
   `repo.head()?.peel_to_tree()?.get_path(path)?.id()`
2. Look up `refs/metadata/comments` tree, navigate to `<blob-oid>/` subtree
3. Enumerate all comment entries under that subtree
4. Read `meta` and `body` blobs for each comment

**Report:** Latency (ms) per file lookup at M=100, 500, 1000 total comments.
This is the most latency-sensitive operation in Forge — it runs every time a
file is opened in the editor.

## Benchmark 4: Relational Metadata Link Traversal

**What it tests:** Bidirectional link lookups — "all comments referencing issue
42", "all reviews referencing issue 42."

**Setup:** Write N link entries under `refs/metadata/links/issues/42/` (N = 10,
100, 500). Each entry is a tree blob with a short metadata payload (~50 bytes).
Also write the reverse direction entries.

**Measured operation:**
1. Read the tip commit of `refs/metadata/links`
2. Navigate the tree to `issues/42/`
3. List all entries (each entry name encodes type and ID, e.g. `comment:abc123`)

**Report:** Latency (ms) at each N. This should be very fast — it is a single
tree listing with no blob reads required.

## Benchmark 5: Approval Lookup by Patch-ID

**What it tests:** "Is this patch approved?" — the merge gate's hot path.

**Setup:** Write P approval entries under `refs/metadata/approvals/<patch-id>/`
for P distinct patch IDs (P = 10, 100, 1000). Each entry is a TOML blob (~100
bytes) keyed by approver fingerprint.

**Measured operation:**
1. Given a patch ID string, check whether `refs/metadata/approvals/<patch-id>/`
   exists and contains at least one entry from a qualifying approver
2. Read the tip commit of `refs/metadata/approvals`
3. Navigate to `<patch-id>/` subtree
4. List entries, check fingerprints against a policy list

**Report:** Latency (ms) at each P. Also measure the miss case (patch ID not
present) — this should be faster than the hit case.

## Benchmark 6: Metadata Auto-Merge

**What it tests:** The server's three-way merge for concurrent metadata writes.

**Setup:** Create a `refs/metadata/comments` ref with an initial commit. Fork
two divergent commits from the same parent — simulate two users each adding a
comment on different blob OIDs simultaneously (non-conflicting paths).

**Measured operation:**
1. Perform a three-way tree merge using `git2::Repository::merge_trees`
2. Write the resulting tree as a new merge commit
3. Update the ref

Also test the conflicting case: two users both resolving the same comment
(both writing to `<comment-id>/resolved`). Verify this produces a conflict
rather than silent data loss.

**Report:** Latency (ms) for the clean merge case and the conflict detection
case at varying comment counts (10, 100, 500 entries in the tree).

## Benchmark 7: Reanchoring

**What it tests:** Comment reanchoring on commit — "move all comments on
changed blobs to their new blob OIDs."

**Setup:** Create a repo with a source file, compute its blob OID, write 10
comments anchored to that blob OID. Then create a new commit that modifies the
file (producing a new blob OID). Simulate blame output as a map of old line
ranges to new line ranges.

**Measured operation:**
1. Given a new commit, diff to find changed files
2. For each changed file, look up comments on the old blob OID
3. For each comment, compute new anchor (apply blame mapping)
4. Write updated metadata entries with new blob OID and line range
5. Commit the metadata update

**Report:** Latency (ms) per reanchoring commit at 1, 10, 50 comments on the
changed file. The question is whether reanchoring on every push is acceptably
fast.

## Scale Targets

These are the numbers Forge must handle without an external index:

| Operation | Target latency | At scale |
|-----------|---------------|----------|
| Issue create | < 50ms | 10,000 issues |
| Issue list (open) | < 200ms | 10,000 issues |
| Comment lookup (file open) | < 20ms | 1,000 comments |
| Link traversal | < 5ms | 500 links |
| Approval lookup | < 10ms | 1,000 approvals |
| Metadata auto-merge | < 100ms | 500 entries |
| Reanchoring | < 500ms | 50 comments/commit |

If any benchmark misses its target, report the actual numbers and the N at
which performance degrades. Do not tune libgit2 settings to make numbers look
better — use defaults. The goal is to know where the limits are, not to hide
them.

## Reporting

Use Criterion's default HTML report output. For each benchmark, report:
- Mean latency and standard deviation
- Throughput where applicable
- The N at which latency first exceeds the target
- Whether the operation scales linearly, sub-linearly, or super-linearly with N

Save results to `target/criterion/`. Include a `RESULTS.md` summarizing
findings and any operations that did not meet targets.
