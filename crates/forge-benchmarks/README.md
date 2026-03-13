# forge-benchmarks

Performance benchmarks for [`git-forge`](../git-forge), measuring whether Git's
object store and ref model can handle Forge's data access patterns at realistic
scale.

These are not micro-benchmarks of libgit2 internals. They test the specific
operations Forge performs — issue creation, comment lookup, approval gating,
metadata merging — at the scale a large active project would produce.

## Running

```sh
cargo bench --package forge-benchmarks
```

HTML reports land in `target/criterion/`. Open
`target/criterion/report/index.html` in a browser for the full interactive
view.

To run a single group:

```sh
cargo bench --package forge-benchmarks -- issue_creation
```

To run in test mode (one iteration each, no timing):

```sh
cargo bench --package forge-benchmarks -- --test
```

## Benchmark descriptions

| # | Group | What it tests |
|---|-------|---------------|
| 1 | `issue_creation` | Counter CAS protocol + ref-per-issue write |
| 2 | `issue_listing` | Ref glob enumeration + meta blob read for all issues |
| 3 | `comment_lookup` | Blob-anchored comment lookup by blob OID |
| 4 | `link_traversal` | Relational link tree listing (`issues/42/*`) |
| 5 | `approval_lookup` | Approval hit and miss by patch-ID |
| 6 | `auto_merge` | Three-way metadata merge (clean and conflicting) |
| 7 | `reanchoring` | Comment reanchoring across a file edit |

Each benchmark is parameterised by scale (N). Inputs mirror production-scale
numbers: up to 10 000 issues, 1 000 comments, 500 links, 1 000 approvals, and
500-entry metadata trees.

## Scale targets

These are the latency targets Forge must meet without an external index.

| Operation | Target | At scale |
|-----------|--------|----------|
| Issue create | < 50 ms | 10 000 issues |
| Issue list (open) | < 200 ms | 10 000 issues |
| Comment lookup (file open) | < 20 ms | 1 000 comments |
| Link traversal | < 5 ms | 500 links |
| Approval lookup | < 10 ms | 1 000 approvals |
| Metadata auto-merge | < 100 ms | 500 entries |
| Reanchoring | < 500 ms | 50 comments/commit |

## Results

> Results below are from a local run on a MacBook Pro M-series (Apple Silicon).
> Criterion uses the default warm-up and sampling settings. Your numbers will
> vary by hardware and filesystem.

Run `cargo bench --package forge-benchmarks` to generate fresh results. The
table below is populated from the last recorded run; update it after each
significant change.

### issue_creation

| N | Mean | Target | Pass? |
|---|------|--------|-------|
| 100 | — | < 50 ms | — |
| 1 000 | — | < 50 ms | — |
| 10 000 | — | < 50 ms | — |

### issue_listing

| N | Mean | Target | Pass? |
|---|------|--------|-------|
| 100 | — | < 200 ms | — |
| 1 000 | — | < 200 ms | — |
| 10 000 | — | < 200 ms | — |

### comment_lookup

| Total comments | Mean | Target | Pass? |
|----------------|------|--------|-------|
| 100 | — | < 20 ms | — |
| 500 | — | < 20 ms | — |
| 1 000 | — | < 20 ms | — |

### link_traversal

| N links | Mean | Target | Pass? |
|---------|------|--------|-------|
| 10 | — | < 5 ms | — |
| 100 | — | < 5 ms | — |
| 500 | — | < 5 ms | — |

### approval_lookup

| N | Variant | Mean | Target | Pass? |
|---|---------|------|--------|-------|
| 10 | hit | — | < 10 ms | — |
| 10 | miss | — | < 10 ms | — |
| 100 | hit | — | < 10 ms | — |
| 100 | miss | — | < 10 ms | — |
| 1 000 | hit | — | < 10 ms | — |
| 1 000 | miss | — | < 10 ms | — |

### auto_merge

| N entries | Variant | Mean | Target | Pass? |
|-----------|---------|------|--------|-------|
| 10 | clean | — | < 100 ms | — |
| 10 | conflict | — | < 100 ms | — |
| 100 | clean | — | < 100 ms | — |
| 100 | conflict | — | < 100 ms | — |
| 500 | clean | — | < 100 ms | — |
| 500 | conflict | — | < 100 ms | — |

### reanchoring

| N comments | Mean | Target | Pass? |
|------------|------|--------|-------|
| 1 | — | < 500 ms | — |
| 10 | — | < 500 ms | — |
| 50 | — | < 500 ms | — |

## Filling in results

After running `cargo bench`, copy mean latencies from the Criterion HTML report
(or stdout) into the tables above. Mark **Pass?** as ✅ or ❌. If a benchmark
misses its target, note the N at which latency first exceeds the target and
describe the scaling behaviour (linear / sub-linear / super-linear).

Do not tune libgit2 settings to make numbers look better. The goal is to know
where the limits are, not to hide them.
