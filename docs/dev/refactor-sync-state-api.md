# Refactor: replace full-state writes with additive sync-state API

## Problem

`save_sync_state` and `write_github_config` both take a complete snapshot and write it, but every caller follows the same pattern:

```text
load → insert one or a few entries → save everything back
```

This creates several issues:

1. **Callers must load first.**
   Every call site does `load_sync_state` then `save_sync_state`, even though it only added a single key.
2. **Full rewrites on each entry.** `import_issues` and `export_issues`
   call `save_sync_state` once at the end, but if an error occurs
   mid-loop, all prior inserts in that run are lost.
3. **Confusing subtree semantics.**
   `save_sync_state({})` is a no-op because "unmentioned subtrees are preserved."
   This is documented and tested, but it's an unintuitive footgun that only exists because the API is snapshot-based.
4. **One git commit per write_config_blob call.** `write_config_blob` is
   called in a loop by `exe.rs:158`, creating N commits for N sigils.

## Proposed API

### `state.rs`

Replace the load-modify-save pair with an additive upsert:

```rust
/// Insert or update a single sync-state entry.
///
/// Creates the sync ref if it does not exist.
pub fn upsert_sync_entry(
    repo: &Repository,
    owner: &str,
    repo_name: &str,
    key: &str,       // e.g. "issues/7"
    value: &str,     // forge OID
) -> Result<()>
```

Keep `load_sync_state` for reads (the lookup helpers and skip-checks still need it).
`save_sync_state` can stay as a low-level primitive but should no longer be the primary write path.

### `config.rs` / `refs.rs`

`write_config_blob` is already additive (single-key upsert), so it's fine.
The problem was `write_github_config` which was calling it in a loop without removing stale entries — that's been fixed on the `test/git-forge-coverage` branch by rebuilding the sigil subtree from scratch.

No API change needed for config unless you want to batch multiple `write_config_blob` calls into a single commit (minor optimization).

## Call sites to update

### `crates/forge-github/src/import.rs`

Current (lines 24, 63, 73):

```rust
let mut state = load_sync_state(repo, &cfg.owner, &cfg.repo)?;
// ...loop...
    state.insert(state_key, created.oid.clone());
// ...end loop...
save_sync_state(repo, &cfg.owner, &cfg.repo, &state)?;
```

After:

```rust
let state = load_sync_state(repo, &cfg.owner, &cfg.repo)?;
// ...loop...
    upsert_sync_entry(repo, &cfg.owner, &cfg.repo, &state_key, &created.oid)?;
// ...end loop...
// no save_sync_state call
```

This also means each imported issue is persisted immediately, so a mid-loop crash doesn't lose all progress.

### `crates/forge-github/src/export.rs`

Same pattern — replace the `state.insert` + final `save_sync_state` with per-entry `upsert_sync_entry` calls (lines 26, 62, 72).

## Implementation notes

- `upsert_sync_entry` should read the current sync ref tree, insert the new blob under `<kind>/<number>`, and commit.
  This is one commit per entry.
  If the commit-per-entry overhead matters later, a batched variant can be added, but for now simplicity wins.
- The existing `save_sync_state` tests should still pass (keep it as an internal/test utility).
  Add new tests for `upsert_sync_entry`: upsert into empty state, upsert into existing state, upsert overwrites existing key.
- `lookup_by_github_id` and `lookup_by_forge_oid` stay unchanged — they
  operate on the loaded HashMap.
- Run `cargo test --workspace` after.

## Scope

This is a refactor — no user-facing behavior changes.
The sync state on-disk format (tree of `<kind>/<number>` blobs under the sync ref) stays the same.
