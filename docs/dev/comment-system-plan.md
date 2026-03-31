# Comment System v2 — Development Plan

## Current state

Comments are `git-chain` appends on per-topic refs:

- `refs/forge/comments/issue/<id>` — one chain per issue
- `refs/forge/comments/review/<id>` — one chain per review
- `refs/forge/comments/object/<blob-oid>` — shared chain for standalone blob comments

Threading uses second parents (reply-to).
Anchoring uses commit message trailers (`Anchor`, `Anchor-Range`, `Anchor-Path`, `Anchor-End`).
Edits use a `Replaces` trailer.
The tree is always empty.

Dependencies: `git-chain` for append/walk, `git-ledger` for issues/reviews.
The `comment` module is ~370 lines in `git-forge/src/comment.rs`.
The `Executor` in `exe.rs` wraps comment operations for issue and review contexts.
The MCP server exposes `list_issue_comments`.
The LSP server reads `object/<blob-oid>` chains for inline diagnostics.
The Neovim plugin shells out to `forge comment`.

### Problems with the current design

1. **Comments are scoped to topics.**
   A blob-anchored comment on a review lives on `refs/forge/comments/review/<id>`.
   If the review is closed or retargeted, the comment's home changes.
   The `migrate_comment` / `Migrated-From` machinery exists because of this coupling.
2. **Single shared chain for object comments.**
   `refs/forge/comments/object` is a single ref.
   Two people commenting on different blobs contend on it.
3. **Write-time reanchoring.**
   The retarget path in `exe.rs` walks trees, diffs blobs, and migrates comments at push time.
   This is fragile (it broke on the plane) and mutates other people's data.
4. **Empty trees.**
   Every comment carries the empty tree.
   Context lines for fuzzy reanchoring aren't stored.

## Target design

Comments are **independent of issues and reviews.**
A comment is a standalone object in git, anchored to code.
Relationships to reviews/issues are relational metadata stored separately.

### Storage: per-thread chain refs

```text
refs/forge/comments/<uuid-v7>
```

Each ref is a **thread**.
The root commit is the top-level comment.
Replies append as first-parent children.
`git log --first-parent <ref>` gives the full conversation in order.

A standalone comment with no replies is a chain of length 1 — identical to the ref-per-comment model.

The ref is communal: any participant can append a reply.
Authorship integrity comes from commit signatures, not ref ownership.

### Comment commit structure

Each commit in a thread chain:

- **First parent:** previous comment in the thread (chronological chain)
- **Tree:** structured data (not empty)
  - `anchor` — blob: TOML with `blob_oid`, `start_line`, `end_line`
  - `context` — blob: surrounding source lines for fuzzy reanchoring
  - `body` — blob: markdown comment text
  - `suggestion` — blob (optional): replacement text for the anchored range
  - `attachments/` — subtree (optional): images, logs, supplementary files
- **Trailers:**
  - `Anchor-Blob: <oid>` — denormalized hint for fast index rebuilds
  - `Comment-Id: <uuid-v7>` — thread identity, same for every commit on the ref
  - `Resolved: true` — present on resolution commits
  - `Replaces: <commit-oid>` — present on edit commits

### Threading

Thread structure is the first-parent DAG.
No second parents needed — replies are sequential within a thread.
Cross-thread references (if ever needed) use `Comment-Id` values, not commit parents.

### Edits

An edit is a new commit appended to the thread with `Replaces: <original-commit-oid>`.
The tree carries the updated body/anchor.
The viewer collapses edits: show the latest `Replaces` chain for each original, with "edited" indicator.

### Resolution

A resolution is a new commit appended to the thread with `Resolved: true`.
Unresolving is another commit without the trailer.
State is determined by the latest commit's trailer presence.

### Anchoring

Blob OID + line range.
Stored in the tree's `anchor` blob as TOML.
The `context` blob stores surrounding lines for fuzzy relocation.

### Reanchoring: read-time only

No write-time reanchoring.
The original anchor is permanent record.
The viewer computes current position:

1. Blame the file → get blob OID per line
2. Direct match on blob OID from the `anchor` entry → comment is current
3. No match → fuzzy match `context` lines against current file content
4. No fuzzy match → comment is detached/orphaned

Cache results in the UI layer (SQLite, in-memory).
Cache invalidates on fetch.

### Discoverability

A derived index ref `refs/forge/index/comments-by-blob` maps blob OIDs → thread UUIDs.
Rebuildable from `git for-each-ref refs/forge/comments/ --format='%(refname)'` plus trailer scanning (`Anchor-Blob`).
Maintained by post-receive hook or background daemon.
The index is a cache — if corrupt, rebuild.

### Fetch

All comments always fetched:

```text
[remote "origin"]
    fetch = +refs/forge/comments/*:refs/forge/comments/*
```

---

## Migration plan

### Phase 0 — Design doc update

Update `docs/design/git-forge-comments.md` to reflect the new design.
Archive the current doc as `git-forge-comments-v1.md`.
The new doc is the authoritative spec; this plan is the execution order.

Replaces: `docs/design/git-forge-comment-plan.md` (mark as historical/superseded).

### Phase 1 — New domain types

**What changes:** `src/comment.rs` — gut and rewrite.

Drop the `git-chain` dependency for comments.
The new `comment` module operates directly on `git2` — `commit_tree`, `update_ref`, `revwalk`.

New types:

```rust
struct Anchor {
    blob_oid: String,
    start_line: u32,
    end_line: u32,
}

struct Comment {
    thread_id: String,       // UUID v7 (= ref suffix)
    commit_oid: String,      // this comment's commit SHA
    body: String,
    author_name: String,
    author_email: String,
    timestamp: i64,
    anchor: Option<Anchor>,
    context_lines: Option<String>,
    resolved: bool,
    replaces: Option<String>, // commit OID of edited original
}
```

Drop `CommitRange` anchor variant.
Drop `migrated_from`.
Drop `reply_to` as a field (threading is positional in the chain).
Drop `Anchor-Path` (path is derived from blame at read time, not stored).

New functions:

- `create_thread(repo, body, anchor, context) -> Result<Comment>` — creates ref + root commit
- `reply(repo, thread_id, body, anchor, context) -> Result<Comment>` — appends to thread ref
- `resolve(repo, thread_id, message) -> Result<Comment>` — appends resolution
- `edit(repo, thread_id, original_oid, new_body, anchor) -> Result<Comment>` — appends edit
- `list_thread(repo, thread_id) -> Result<Vec<Comment>>` — walks first-parent
- `list_all_threads(repo) -> Result<Vec<String>>` — `for-each-ref` on prefix

Tree building helper: takes anchor TOML + context + body + optional suggestion, returns a tree OID.

**Acceptance:** `cargo check -p git-forge` passes.
Old tests will break — that's expected.

### Phase 2 — Tests

Rewrite `tests/comment.rs` against the new API.
Test cases:

1. `create_thread` produces a ref and a root commit with correct tree entries
2. `reply` appends a commit whose first parent is the previous tip
3. `list_thread` returns comments in chronological order
4. `resolve` sets the `Resolved` trailer; subsequent check sees resolved state
5. `edit` carries `Replaces` trailer pointing at original commit OID
6. Two threads on different blob OIDs don't interfere (no shared ref contention)
7. `list_all_threads` returns all thread UUIDs
8. Tree contents round-trip: anchor TOML, context blob, body blob all survive write → read
9. Thread with replies from "different authors" (different git signatures) works

**Acceptance:** `cargo test -p git-forge` passes.

### Phase 3 — Executor migration

Update `exe.rs`:

- Remove `migrate_carry_forward_comments` and all retarget-triggered comment migration logic.
- Remove the three-way chain routing (`review_comment_chain_ref` choosing between review/object chains).
- `add_issue_comment` → `create_thread` with no anchor (issue comments are unanchored discussion).
- `add_review_comment` → `create_thread` with blob anchor.
  The review's OID is relational metadata, not part of the comment ref.
- `reply_*` → `reply` by thread UUID.
- `resolve_*` → `resolve` by thread UUID.
- `list_issue_comments` → query by relational metadata (issue OID → thread UUIDs).
  **This requires Phase 4.**

Intermediate state: issue/review comment listing may be temporarily broken until relational metadata is wired.
Accept this — keep old code paths behind a feature flag if needed, or just let them fail gracefully.

**Acceptance:** `cargo check -p git-forge` passes.
Core comment CRUD works via CLI.

### Phase 4 — Relational metadata

Comments are independent.
Linking a comment to a review or issue is a separate concern.

Options (in order of simplicity):

1. **Trailer on the root commit:** `Issue: <oid>` or `Review: <oid>`.
   The comment "knows" what prompted it, but only as metadata.
   Discovery: scan all thread root commits for the trailer.
   This is O(threads), not O(comments), which is fine.
2. **Entries in the issue/review ledger:** the issue's ledger entry gets a `comments/<thread-uuid>` field.
   Discovery is a single ref read.
   But now creating a comment requires writing to two refs (comment ref + issue ref).

Recommend option 1.
One write per comment.
Discovery cost is acceptable — thread count is orders of magnitude less than comment count.

Add a `Related-To: <oid>` trailer on thread root commits.
Query: `git for-each-ref refs/forge/comments/ --format='%(refname) %(trailers:key=Related-To,valueonly)'` filtered by target OID.

Update `list_issue_comments` and `list_review_comments` in `Executor` to use this query.

**Acceptance:** `cargo test -p git-forge` passes. `forge comment list --issue <id>` works.

### Phase 5 — CLI update

Update `cli.rs` (the clap definitions) and the dispatch in `exe.rs`:

```text
forge comment create [--blob <oid>] [--lines <start>-<end>] [--issue <id>] [--review <id>] --body <text>
forge comment reply <thread-uuid> --body <text>
forge comment resolve <thread-uuid> [--message <text>]
forge comment edit <thread-uuid> <commit-oid> --body <text>
forge comment list [--issue <id>] [--review <id>] [--blob <oid>]
forge comment show <thread-uuid>
```

The `--blob` flag resolves via `git rev-parse HEAD:<path>` in the CLI layer (convenience).
The `--lines` flag requires `--blob`.

Context lines: when `--blob` and `--lines` are provided, the CLI reads the blob content and extracts ±3 surrounding lines automatically.
The user never provides context manually.

**Acceptance:** `cargo test -p git-forge` passes.
All CLI paths exercised in integration tests.

### Phase 6 — MCP server update

Update `forge-mcp/src/comment.rs`:

- `list_issue_comments` → uses `Related-To` trailer query
- Add `list_review_comments` (same pattern)
- Add `list_blob_comments` — queries the derived index (or falls back to scanning `Anchor-Blob` trailers)
- Add `create_comment` tool — wraps `create_thread`
- Add `reply_comment` tool — wraps `reply`
- Add `resolve_comment` tool — wraps `resolve`

**Acceptance:** MCP server starts.
Tools return correct JSON for all operations.

### Phase 7 — LSP server update

Update `forge-lsp/src/*.rs`:

The LSP currently reads `refs/forge/comments/object/<blob-oid>` chains.
Switch to scanning all comment threads via `Anchor-Blob` trailer or derived index, filtered to the blob OID of the currently open file.

For files not at HEAD (diffing old revisions), the LSP resolves the file's blob OID at that revision and queries comments anchored to it.

**Acceptance:** Opening a file in an editor shows inline comment diagnostics for comments on that file's blob OID.

### Phase 8 — Neovim plugin update

Update `extensions/forge-nvim/lua/forge.lua`:

The plugin currently shells out to `forge comment --blob <oid> --lines <start>-<end>`.
Update to match the new CLI:

```text
forge comment create --blob <oid> --lines <start>-<end> --body <text>
```

No structural change to the plugin — just command string updates.

**Acceptance:** Visual-select lines in Neovim, `<leader>fc`, enter comment text — comment is created with correct anchor.

### Phase 9 — Derived index

Implement the `refs/forge/index/comments-by-blob` index:

- A commit whose tree is a fanout by blob OID prefix (2-char fanout), with each leaf being a blob containing newline-separated thread UUIDs.
- Rebuild function: `git for-each-ref refs/forge/comments/ --format='%(refname) %(trailers:key=Anchor-Blob,valueonly)'` → parse → build tree → commit on index ref.
- Incremental update: on new comment refs, add entries.
  On deleted refs, remove entries.
- The index is only used for `list_blob_comments`.
  Everything else works without it.

This is an optimization.
Ship it after the core is stable.
The system works without it — queries just scan trailers.

**Acceptance:** `forge index rebuild` creates the index. `forge comment list --blob <oid>` uses it when present, falls back to scan when absent.

### Phase 10 — Cleanup

- Remove `git-chain` from `Cargo.toml` dependencies (if no other module uses it).
- Remove `migrate_comment`, `migrated_from` field, `Migrated-From` trailer.
- Remove `review_comment_chain_ref` routing logic.
- Remove `OBJECT_COMMENTS_PREFIX` constant.
- Update `ISSUE_COMMENTS_PREFIX` and `REVIEW_COMMENTS_PREFIX` — these are no longer needed.
  All comments live under `refs/forge/comments/<uuid>`.
- Delete `docs/design/git-forge-comment-plan.md` or mark superseded.
- Update `README.md` if it references the old comment model.

**Acceptance:** `cargo test --workspace` passes.
No references to `git-chain` in the comment path.
`git grep 'OBJECT_COMMENTS_PREFIX\|REVIEW_COMMENTS_PREFIX\|ISSUE_COMMENTS_PREFIX\|migrate_comment\|Migrated-From'` returns nothing.

---

## Ordering and dependencies

```text
Phase 0 (doc)
  │
Phase 1 (types) ──→ Phase 2 (tests)
  │
Phase 3 (executor) ──→ Phase 4 (relational metadata)
                          │
                    Phase 5 (CLI) ──→ Phase 6 (MCP)
                          │              │
                    Phase 7 (LSP)   Phase 8 (Neovim)
                          │
                    Phase 9 (index) ──→ Phase 10 (cleanup)
```

Phases 1–5 are the critical path.
Phases 6–8 are independent of each other.
Phase 9 is an optimization that can ship whenever.
Phase 10 is last.

## Non-goals for this plan

- **GitHub sync.**
  `forge-github` syncs issues and reviews.
  Comment sync is a separate concern and not part of this migration.
- **Web UI.**
  The SQLite-indexed web UI will consume comment data via the same `git for-each-ref` + tree-read path.
  No special accommodation needed.
- **Approval system.**
  Approvals are annotations on patch-ids, not comments.
  Separate design, separate plan.
