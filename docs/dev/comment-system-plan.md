# Comment System v2 — Development Plan

## What's changing and why

The comment system's **commit structure** is fine: `git-chain` appends, second-parent threading, trailer-based anchoring.
What's wrong is the **ref topology** and the **write-time migration machinery** built around it.

### Problems

1. **Comments are scoped to topics.**
   Blob-anchored comments on a review live on `refs/forge/comments/review/<id>` or `refs/forge/comments/object/<blob-oid>`.
   Retargeting a review triggers comment migration between chains.
   This migration broke on the plane.
2. **The three-way routing in `review_comment_chain_ref`** decides whether a comment goes to `review/<id>` or `object/<blob-oid>` based on anchor type.
   Fragile and surprising.
3. **Write-time reanchoring** (`migrate_carry_forward_comments`, `migrate_delta`, `migrate_comment`) rewrites comments into new chains when a review's target changes.
   This mutates data, races on concurrent pushes, and is the source of the bugs.
4. **Empty trees.**
   Every comment carries the empty tree.
   Context lines for fuzzy reanchoring aren't stored anywhere, so write-time migration is the only option.

### What stays

- `git-chain` as the append/walk primitive.
- Second-parent threading.
- `git-ledger` for issues and reviews (untouched).

### What changes

| Aspect | v1 | v2 |
|---|---|---|
| Ref topology | `refs/forge/comments/{issue,review,object}/<id>` | `refs/forge/comments/<uuid-v7>` |
| Comment ownership | Scoped to issue/review/blob | Independent. Linked via anchor to any git object. |
| One chain per... | Topic (issue, review, or blob OID) | Thread (conversation) |
| Anchor target | Blob OID or commit range | Any git object (blob, commit, tree) |
| Tree content | Always empty | `anchor` + `context` + `body` + `anchor-content` |
| Reanchoring | Write-time migration on retarget | Read-time via blame + fuzzy match on `context` blob |
| Comment migration | `migrate_comment` / `Migrated-From` | Deleted. Not needed. |
| Discovery | Scan per-topic chain refs | Server-rebuilt signed `comments-by-object` index |
| Linking to issues/reviews | Separate ref namespaces | Anchor to the issue/review's commit OID |

### Key design principles

**Every comment anchors to a git object.**
An issue comment anchors to the issue's root commit OID.
A review comment anchors to the review's head commit OID.
An inline code comment anchors to a blob OID with a line range.
A directory-level comment anchors to a tree OID.
There is one anchor mechanism, one index, and one query path: `find_threads_by_object(oid)`.

**Second parent means "in response to."**
Every second parent has exactly one structural meaning: this commit is a response to that commit.
Two trailers modify display behavior:

- `Resolved: true` — the viewer treats this as a resolution of the target comment's thread, not a standalone reply.
- `Replaces: <oid>` — the viewer treats this as an edit of the target comment, hiding the original and showing the edit in its place.

A commit with a second parent and neither trailer is a plain reply.

**GC safety via `anchor-content` tree entry.**
The anchored git object is inserted into the comment's tree by OID: blobs with mode `0o100644`, trees with mode `0o040000`, commits with mode `0o160000` (gitlink).
This creates a reachability edge in git's object graph, preventing GC of the anchored object.
This is the same pattern used by `fixup_pin_entries` in `review.rs` for pinning review target objects.

---

## Current code inventory

Files that need changes:

| File | Role | Change |
|---|---|---|
| `src/comment.rs` | Domain types, chain operations | Modify types, add tree building, drop `migrate_comment` |
| `src/refs.rs` | Ref prefix constants | Replace three comment prefixes with one |
| `src/exe.rs` | CLI execution logic | Gut migration logic, simplify routing |
| `src/cli.rs` | Clap definitions | Update comment subcommands |
| `tests/comment.rs` | Integration tests | Rewrite for new API |
| `forge-mcp/src/comment.rs` | MCP tools | Update to new query pattern |
| `forge-lsp/src/*.rs` | LSP diagnostics | Update ref queries |
| `extensions/forge-nvim/lua/forge.lua` | Neovim plugin | Update CLI invocation strings |
| `docs/design/git-forge-comments.md` | Design doc | Rewrite |

---

## Phase 1 — Update design doc

Archive `docs/design/git-forge-comments.md` as `docs/design/git-forge-comments-v1.md`.

Write new `docs/design/git-forge-comments.md` describing the full v2 design as outlined in this plan's preamble.
Include:

- Per-thread chain refs: `refs/forge/comments/<uuid-v7>`
- `git-chain` append/walk, second-parent semantics, trailer semantics
- Generalized anchor targeting any git object
- Non-empty tree structure: `anchor`, `context`, `body`, `anchor-content`
- Read-time reanchoring for blob anchors
- Server-rebuilt signed `comments-by-object` index
- Fetch strategy: all comments always fetched

Mark `docs/design/git-forge-comment-plan.md` header as superseded (it already says COMPLETED; add a note pointing to this plan).

**Acceptance:** Docs compile/render.
No code changes.

---

## Phase 2 — Ref constants

In `src/refs.rs`:

1. Add:

   ```rust
   /// Ref prefix for comment threads.
   pub const COMMENTS_PREFIX: &str = "refs/forge/comments/";
   /// Index ref mapping object OIDs to comment thread UUIDs.
   pub const COMMENTS_INDEX: &str = "refs/forge/index/comments-by-object";
   ```

2. Keep the old constants (`ISSUE_COMMENTS_PREFIX`, `REVIEW_COMMENTS_PREFIX`, `OBJECT_COMMENTS_PREFIX`) temporarily — they'll be removed in the cleanup phase.

**Acceptance:** `cargo check -p git-forge` passes.

---

## Phase 3 — Comment types and tree building

In `src/comment.rs`:

1. **Generalize `Anchor`.**
   Replace the `Object`/`CommitRange` enum with:

   ```rust
   pub struct Anchor {
       /// OID of the anchored git object (blob, commit, or tree).
       pub oid: String,
       /// Line range — only meaningful for blob anchors.
       pub start_line: Option<u32>,
       pub end_line: Option<u32>,
   }
   ```

   Keep the old `Anchor` enum temporarily as `LegacyAnchor`.
   Rename it, add the new struct, update call sites incrementally.

2. **Add fields to `Comment`:**

   ```rust
   pub struct Comment {
       // ... existing fields ...
       pub context_lines: Option<String>,
       pub thread_id: Option<String>,  // UUID from Comment-Id trailer
   }
   ```

   The `migrated_from` field stays for now (removed in cleanup).

3. **Add tree building.**
   New function:

   ```rust
   pub fn build_comment_tree(
       repo: &Repository,
       body: &str,
       anchor: Option<&Anchor>,
       context_lines: Option<&str>,
   ) -> Result<git2::Oid>
   ```

   Builds a tree with entries:
   - `body` — blob containing the markdown text
   - `anchor` — blob containing TOML: `oid = "..."\n` with optional `start_line` and `end_line` fields
   - `context` — blob containing surrounding source lines (only for blob anchors with a line range, omitted otherwise)
   - `anchor-content` — the actual anchored git object, inserted by OID.
     Determine mode from object type: blob → `0o100644`, tree → `0o040000`, commit → `0o160000` (gitlink).
     Use `repo.find_object(oid, None)?.kind()` to determine type.

4. **Add trailer constants:**

   ```rust
   const TRAILER_COMMENT_ID: &str = "Comment-Id";
   ```

   Add to `KNOWN_TRAILER_KEYS`.
   Update `parse_trailers` to extract it.
   Update `comment_from_chain_entry` to populate the new `Comment` fields.
   The existing `Anchor` trailer key stays — it is the denormalized hint for index builds.

5. **`Anchor` trailer on every commit.**
   When building the trailer block for any comment (root, reply, resolve, edit), always include `Anchor: <oid>` if an anchor is provided.
   This means the index can find threads by reading only the tip commit of each ref (via `git for-each-ref`), without walking to the root.
   Replies that have their own anchor (e.g., a reply pointing at a different line) carry their own `Anchor` value; replies that don't specify an anchor inherit and repeat the root's anchor.

6. **New ref helper:**

   ```rust
   pub fn comment_thread_ref(thread_id: &str) -> String {
       format!("{}{thread_id}", crate::refs::COMMENTS_PREFIX)
   }
   ```

7. **New functions** (wrap `git-chain` operations with new tree and trailers):

   ```rust
   /// Create a new comment thread. Returns (thread_id, root_comment).
   pub fn create_thread(
       repo: &Repository,
       body: &str,
       anchor: Option<&Anchor>,
       context_lines: Option<&str>,
   ) -> Result<(String, Comment)>
   ```

   Generates a UUID v7, builds the comment tree via `build_comment_tree`, formats trailers (`Anchor: <oid>`, `Comment-Id: <uuid>`), calls `repo.append(ref_name, message, tree, None)`.

   ```rust
   /// Append a reply to a thread.
   pub fn reply_to_thread(
       repo: &Repository,
       thread_id: &str,
       body: &str,
       reply_to_oid: &str,
       anchor: Option<&Anchor>,
       context_lines: Option<&str>,
   ) -> Result<Comment>
   ```

   Builds tree, formats trailers (same `Comment-Id`, plus `Anchor` — either the reply's own anchor or the root's anchor), resolves `reply_to_oid` to full OID, calls `repo.append(ref_name, message, tree, Some(parent))`.

   ```rust
   /// Append a resolution to a thread.
   pub fn resolve_thread(
       repo: &Repository,
       thread_id: &str,
       reply_to_oid: &str,
       message: Option<&str>,
   ) -> Result<Comment>
   ```

   Second parent is the comment being resolved.
   Trailer includes `Resolved: true`.
   Inherits root's `Anchor`.

   ```rust
   /// Append an edit to a thread.
   pub fn edit_in_thread(
       repo: &Repository,
       thread_id: &str,
       original_oid: &str,
       new_body: &str,
       anchor: Option<&Anchor>,
       context_lines: Option<&str>,
   ) -> Result<Comment>
   ```

   Second parent is the comment being edited.
   Trailer includes `Replaces: <oid>`.
   Inherits anchor from original unless a new anchor is provided.

   ```rust
   /// List all comments in a thread (first-parent walk).
   pub fn list_thread_comments(repo: &Repository, thread_id: &str) -> Result<Vec<Comment>>
   ```

   ```rust
   /// List all thread UUIDs in the repository.
   pub fn list_all_thread_ids(repo: &Repository) -> Result<Vec<String>>
   ```

   ```rust
   /// Find threads containing comments anchored to a given object OID.
   /// Uses the signed index if present, falls back to scanning tip commit trailers.
   pub fn find_threads_by_object(repo: &Repository, oid: &str) -> Result<Vec<String>>
   ```

   The index maps object OIDs to thread UUIDs across all comments in all threads (not just roots).
   The fallback scan reads the `Anchor` trailer from the tip commit of each thread ref — since every commit carries the trailer, the tip is sufficient to find at least one anchor per thread.
   For full coverage (threads where a reply anchors to a different object than the root), the scan must walk all commits.
   This is O(total comments) in the worst case, which the index avoids.

8. **Keep all existing functions** (`add_comment`, `add_reply`, `resolve_comment`, `edit_comment`, `list_comments`, `migrate_comment`, etc.) intact for now.
   Mark old functions `#[deprecated]`.

9. **Add UUID v7 dependency.**
   Add `uuid = { version = "1", features = ["v7"] }` to the workspace `[workspace.dependencies]` table in root `Cargo.toml`, and add `uuid = { workspace = true }` to `crates/git-forge/Cargo.toml` under `[dependencies]`.

**Acceptance:** `cargo check -p git-forge` passes.
Old tests still compile.

---

## Phase 4 — Tests for new API

Add new tests in `tests/comment.rs` (append to existing file, don't delete old tests yet):

1. `create_thread_produces_ref` — `create_thread` creates `refs/forge/comments/<uuid>`, root commit has non-empty tree with `body`, `anchor`, `context` entries.
2. `thread_tree_roundtrip` — create thread with blob anchor + context, read back via `list_thread_comments`, verify all fields survive.
3. `anchor_content_gc_safety_blob` — create thread anchored to a blob.
   Verify the comment's tree contains an `anchor-content` entry whose OID matches the anchored blob.
4. `anchor_content_gc_safety_commit` — create thread anchored to a commit OID.
   Verify `anchor-content` is a gitlink (`0o160000`) with the correct OID.
5. `anchor_content_gc_safety_tree` — create thread anchored to a tree OID.
   Verify `anchor-content` is a tree entry with the correct OID.
6. `reply_appends_to_chain` — reply's first parent is the chain tip, second parent is the comment replied to.
   No special trailers → plain reply.
7. `resolve_sets_trailer` — resolution commit has `Resolved: true` and second parent pointing at the resolved comment.
8. `edit_sets_replaces` — edit commit has `Replaces: <oid>` and second parent pointing at the edited comment.
9. `list_thread_returns_chronological` — multiple replies, verify order.
10. `find_threads_by_object_blob` — create threads anchored to different blobs, verify `find_threads_by_object` returns only matching threads.
11. `find_threads_by_object_commit` — create a thread anchored to a commit OID (simulating an issue comment), verify lookup.
12. `two_threads_no_contention` — create two threads on different objects, verify independence.
13. `comment_id_trailer_consistent` — all commits in a thread carry the same `Comment-Id`.
14. `anchor_trailer_on_every_commit` — create thread, add reply, add resolution.
    Verify every commit in the chain has the `Anchor` trailer.
15. `anchor_to_issue_commit` — create an issue via `Store`, then create a comment thread anchored to the issue's root commit OID. `find_threads_by_object(issue_oid)` returns the thread.

**Acceptance:** `cargo test -p git-forge` passes (both old and new tests).

---

## Phase 5 — Comments-by-object index

Implement `refs/forge/index/comments-by-object`:

**Structure:** A signed commit → tree with 2-character fanout by object OID prefix.
Each leaf blob contains newline-separated thread UUIDs.

```text
refs/forge/index/comments-by-object → signed commit → tree
├── 7e/
│   └── 3f1a2b... → blob: "019538a7-...\n019538b2-...\n"
├── af/
│   └── 3b2c4d... → blob: "019538c1-...\n"
...
```

The index maps object OIDs to thread UUIDs across **all comments in all threads**, not just roots.
A thread where the root anchors to blob A and a reply anchors to blob B produces entries for both A and B.

**Functions:**

```rust
/// Rebuild the index from scratch by scanning all comment thread refs.
pub fn rebuild_comments_index(repo: &Repository) -> Result<()>
```

For each ref under `refs/forge/comments/*`, walk all commits and read the `Anchor` trailer from each.
Build the fanout tree mapping each unique anchor OID to the set of thread UUIDs containing a comment anchored to it.
Commit to `refs/forge/index/comments-by-object`.

If a GPG signing key is configured (`user.signingkey` in git config), sign the index commit.
This allows clients to verify the index was built by a trusted source (server, CI, `narvi`) without rebuilding locally.

```rust
/// Look up thread UUIDs by object OID using the index.
/// Returns None if the index ref doesn't exist.
pub fn index_lookup(repo: &Repository, oid: &str) -> Result<Option<Vec<String>>>
```

**Update `find_threads_by_object`:** try `index_lookup` first.
If the index ref exists and lookup succeeds, return the result.
If the index ref doesn't exist, fall back to scanning all thread tip commits' `Anchor` trailers.
Note: the fallback scan of tip commits only catches the most recent anchor per thread.
For full coverage without the index, the scan must walk all commits in all threads — this is expensive and acceptable only as a degraded mode.

**Server integration:** The index is rebuilt by the server's post-receive hook after each push that touches `refs/forge/comments/*`.
It is NOT incrementally updated by clients.
Clients consume the index read-only.
If the index is stale (new comments since last rebuild), the fallback scan covers the gap.

**CLI:** Add `forge index rebuild` command.

**Acceptance:** `cargo test -p git-forge` passes.
`forge index rebuild` creates the index ref.
`find_threads_by_object` uses the index when present, falls back when absent.
Deleting the index ref doesn't break anything.

---

## Phase 6 — Executor migration

In `src/exe.rs`:

1. **Delete** `migrate_carry_forward_comments`, `migrate_delta`, `map_range_through_hunks`, and `parse_line_range` functions.

2. **Delete** `review_comment_chain_ref` method.

3. **Update `retarget_review`** to NOT call migration:

   ```rust
   pub fn retarget_review(&self, reference: &str, new_head: &str) -> Result<Review> {
       let resolved_head = resolve_to_oid(&self.repo, new_head)?;
       let (_, review) = self.store().retarget_review(reference, &resolved_head)?;
       Ok(review)
   }
   ```

   Update the return type (no longer returns `(Review, usize)`).
   Update all call sites in `exe.rs` that destructure the migrated count.
   Drop the "Migrated N carry-forward comment(s)" message.

4. **Add new comment executor methods:**

   ```rust
   /// Create a comment thread anchored to any git object.
   pub fn create_comment(
       &self,
       body: &str,
       anchor: Option<&Anchor>,
       context_lines: Option<&str>,
   ) -> Result<(String, Comment)>
   ```

   ```rust
   pub fn reply_comment(
       &self,
       thread_id: &str,
       body: &str,
       reply_to_oid: &str,
       anchor: Option<&Anchor>,
       context_lines: Option<&str>,
   ) -> Result<Comment>
   ```

   ```rust
   pub fn resolve_comment_thread(
       &self,
       thread_id: &str,
       reply_to_oid: &str,
       message: Option<&str>,
   ) -> Result<Comment>
   ```

   ```rust
   /// List all comment threads anchored to a git object.
   /// The oid can be any git object: blob, commit, tree.
   pub fn list_comments_on(&self, oid: &str) -> Result<Vec<Comment>>
   ```

   Calls `find_threads_by_object(repo, oid)`, then `list_thread_comments` for each thread.
   Flattens and sorts by timestamp.

5. **Context line extraction helper:**

   ```rust
   fn extract_context(repo: &Repository, blob_oid: &str, start: u32, end: u32) -> Result<String>
   ```

   Reads the blob, extracts ±3 surrounding lines.
   Called automatically inside `create_comment` and `reply_comment` when the anchor targets a blob with a line range.
   The caller never provides context manually.

6. **Display ID resolution helper:**

   ```rust
   fn resolve_anchor_spec(&self, spec: &str) -> Result<String>
   ```

   Accepts:
   - A raw 40-character hex OID → returned as-is
   - A `HEAD:<path>` spec → resolved via `git rev-parse` to blob OID
   - `issue:<display-id>` → resolved to the issue's root commit OID via the issue index
   - `review:<display-id>` → resolved to the review's root commit OID via the review index

   Used by the CLI dispatch layer so users don't need to manually resolve OIDs.

7. **Keep old executor methods** temporarily.
   Mark `#[deprecated]`.

**Acceptance:** `cargo check -p git-forge` passes.
The `retarget_review` path no longer migrates comments.

---

## Phase 7 — CLI update

In `src/cli.rs`, update the `Comment` subcommand:

```text
forge comment create --on <spec> [--lines <start>-<end>] [--body <text>]
forge comment reply <thread-id> --to <oid> [--body <text>]
forge comment resolve <thread-id> --comment <oid> [--message <text>]
forge comment edit <thread-id> --comment <oid> --body <text>
forge comment list --on <spec>
forge comment show <thread-id>
forge index rebuild
```

The `--on` flag accepts any anchor spec, resolved via `resolve_anchor_spec`:

- `--on abc123def...` — raw OID (any object type)
- `--on HEAD:src/main.rs` — resolves to blob OID
- `--on issue:3` — resolves to issue #3's root commit OID
- `--on review:5` — resolves to review #5's root commit OID

When `--on` resolves to a blob and `--lines` is provided, context extraction is automatic.

Examples:

```bash
# Inline code comment
forge comment create --on HEAD:src/main.rs --lines 42-47 --body "Off-by-one here"

# Comment on an issue
forge comment create --on issue:3 --body "I think we should reconsider"

# Comment on a review (overall, not on a specific line)
forge comment create --on review:5 --body "LGTM"

# List all comments on a file
forge comment list --on HEAD:src/main.rs

# List all comments on an issue
forge comment list --on issue:3
```

Wire dispatch in `exe.rs` to call the new executor methods from Phase 6.

**Acceptance:** `cargo test -p git-forge` passes. `forge comment create --help` shows new flags.

---

## Phase 8 — Integration test updates

Update `tests/comment.rs`:

1. Delete old tests that reference `issue_comment_ref`, `review_comment_ref`, `object_comment_ref`, `migrate_comment`.
2. Update executor tests to use the new API.
3. Add end-to-end test: create issue → `create_comment` anchored to issue's OID → `list_comments_on(issue_oid)` → verify.
4. Add end-to-end test: create review → `create_comment` anchored to blob OID → `list_comments_on(blob_oid)` → verify.
5. Add test: retarget review → verify comments are NOT migrated, still anchored to original blob OID, still discoverable.
6. Add test: `rebuild_comments_index` → `find_threads_by_object` returns correct results from index.
7. Add test: delete index ref → `find_threads_by_object` falls back to scan, still returns correct results.
8. Add test: `resolve_anchor_spec` resolves `issue:<id>`, `review:<id>`, `HEAD:<path>`, and raw OIDs correctly.

Update `tests/exe_fixes.rs` if it references migration logic or the old comment routing.

**Acceptance:** `cargo test -p git-forge` passes with zero `#[deprecated]` warnings in test code.

---

## Phase 9 — MCP server update

In `forge-mcp/src/comment.rs`:

1. Replace `list_issue_comments` tool with `list_comments_on` — accepts any OID.
   The MCP caller (LLM agent) resolves issue/review display IDs to OIDs before calling, or provides raw OIDs directly.
2. Add `create_comment` tool — accepts body, anchor OID, optional line range.
3. Add `reply_comment` tool — accepts thread ID, reply-to OID, body.
4. Add `resolve_comment` tool — accepts thread ID, comment OID.
5. Add `show_thread` tool — accepts thread ID, returns all comments in the thread.

One `list_comments_on` tool handles all cases.
No separate issue/review/blob comment tools.

**Acceptance:** `cargo check -p forge-mcp` passes.
MCP server starts and tools return correct JSON.

---

## Phase 10 — LSP server update

In `forge-lsp/src/*.rs`:

The LSP currently reads `refs/forge/comments/object/<blob-oid>`.
Change to:

1. On file open / file change: resolve file path to blob OID via `git rev-parse HEAD:<path>`.
2. Call `find_threads_by_object(repo, &blob_oid)` to get thread IDs.
3. For each thread, `list_thread_comments` and filter for unresolved comments with matching anchor line ranges.
4. Publish diagnostics as before.

For v2 launch, only the current blob OID is queried.
Historical blob coverage (comments on previous versions of the file, discoverable via `git log --follow`) is deferred.

**Acceptance:** Opening a file with blob-anchored comments shows inline diagnostics.

---

## Phase 11 — Neovim plugin update

In `extensions/forge-nvim/lua/forge.lua`:

Update the shell command to:

```lua
forge comment create --on <oid> --lines <start>-<end> --body <text>
```

where `<oid>` is resolved from `git rev-parse HEAD:<relative-path>` as before.

No structural change to the plugin.
The `<leader>fc` keymap, visual-mode selection, floating input window, and git root detection all stay the same.

**Acceptance:** Visual-select lines in Neovim, `<leader>fc`, enter text → comment created on correct blob + line range.

---

## Phase 12 — Cleanup

1. Delete deprecated functions: `add_comment`, `add_reply`, `resolve_comment`, `edit_comment` (the old per-topic-chain versions), `migrate_comment`.
2. Delete `LegacyAnchor` / old `Anchor` enum.
3. Delete `migrated_from` field from `Comment`.
4. Delete old ref constants: `ISSUE_COMMENTS_PREFIX`, `REVIEW_COMMENTS_PREFIX`, `OBJECT_COMMENTS_PREFIX`.
5. Delete old ref helpers: `issue_comment_ref`, `review_comment_ref`, `object_comment_ref`.
6. Delete old executor methods: `add_issue_comment`, `reply_issue_comment`, `resolve_issue_comment`, `add_review_comment`, `reply_review_comment`, `resolve_review_comment`, `list_issue_comments`, `list_review_comments`.
7. Delete `review_target_files` helper in `exe.rs`.
8. Remove `Migrated-From` from `KNOWN_TRAILER_KEYS`.
9. Verify: `git grep -n 'OBJECT_COMMENTS_PREFIX\|REVIEW_COMMENTS_PREFIX\|ISSUE_COMMENTS_PREFIX\|migrate_comment\|Migrated-From\|migrated_from\|review_comment_chain_ref\|migrate_carry_forward\|migrate_delta\|Related-To'` — must return nothing.

**Acceptance:** `cargo test --workspace` passes.
No deprecated warnings.
Grep returns empty.

---

## Dependency graph

```text
Phase 1 (doc)
  │
Phase 2 (refs) ──→ Phase 3 (types + tree building) ──→ Phase 4 (tests)
                                                           │
                                                     Phase 5 (index) ──→ Phase 6 (executor) ──→ Phase 7 (CLI) ──→ Phase 8 (integration tests)
                                                                                                     │
                                                                                               ┌─────┼─────┐
                                                                                               │     │     │
                                                                                            Ph 9  Ph 10 Ph 11
                                                                                            (MCP) (LSP) (nvim)
                                                                                               │     │     │
                                                                                               └─────┼─────┘
                                                                                                     │
                                                                                               Phase 12 (cleanup)
```

Phases 9, 10, 11 are independent of each other and can be done in any order or in parallel.

## Non-goals

- **GitHub sync.**
  Comment sync via `forge-github` is a separate concern.
- **Web UI.**
  Consumes the same `find_threads_by_object` query path.
  No special accommodation.
- **Approvals.**
  Annotations on patch-ids, not comments.
  Separate design.
- **Cross-file reanchoring.**
  Intentionally not supported.
  Same-file blame only.
  Cross-file moves orphan the comment.
- **Historical blob coverage in LSP.**
  Querying comments across all historical versions of a file is deferred. v2 launch queries current blob OID only.
