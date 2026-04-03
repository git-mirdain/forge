# Forge MVP Development Plan

## Goal

An agent reviews code on a branch and leaves comments.
A developer opens the review in their editor, sees comments inline, resolves or replies, and approves — all without leaving the editor or touching GitHub.

## Demo Screenplay

1. Agent (via forge-mcp in Zed or Claude Code) is asked: "Review the changes on `feature-branch`."
2. Agent reads the diff, leaves blob-anchored comments via MCP tools.
3. Developer runs `forge review start <ref>` — a worktree is created, editor opens it.
4. forge-lsp fires `publish_all_diagnostics` on init.
   Developer sees all comments as diagnostics.
5. Developer triggers `forge.review.open` from code action — all files with unresolved comments open via `showDocument`.
6. Developer reads comments, uses code actions to reply or resolve.
7. Developer runs `forge review approve` in the integrated terminal (context inferred from worktree HEAD).
8. Developer runs `forge review finish` — worktree removed, back to main project.

All data is git refs.
No network required for the review itself.

---

## Work Items

### 1. Config-driven sync scope

**Why:** Reviews stay local; issue sync (including issue comments) continues.
The data model and code for review sync remain intact — config controls what runs.

**Changes:**

- Add a `sync` config field (e.g. `sync = ["issues"]`) that controls which object types forge-server syncs.
  Default to `["issues"]`.
  `"all"` or `["issues", "reviews"]` re-enables full sync without code changes.
- `crates/forge-server/src/main.rs`: Gate sync calls on the config value — call `import_issues()`/`export_issues()` when only issues are enabled, `import_all()`/`export_all()` when all are enabled.
  The underlying functions already exist separately in `forge-github`.
- Update log messages to reflect the configured scope.

**Validation:** Run `forge-server --once` against a repo with a GitHub config.
Confirm issues sync, reviews do not.

---

### 2. Change worktree path convention to sibling `@` directory

**Why:** Makes it visually obvious which directory is the main repo vs a review context.
Sorts adjacently in file managers and editor project pickers.

**Current behavior:** `forge review checkout <ref>` creates worktree at `../<repo>.review/<label>`.

**New behavior:** Worktree at `../<repo>@<branch-or-label>`.

**Changes:**

- `crates/git-forge/src/exe.rs`, `checkout_review()`: Change `default_path` computation from:

  ```text
  ../<repo_name>.review/<safe_label>
  ```

  to:

  ```text
  ../<repo_name>@<safe_label>
  ```

- `done_review()`: Same path derivation change for cleanup.
- Update any tests that assert on worktree paths.

---

### 3. Rename CLI subcommands: `checkout` → `start`, `done` → `finish`

**Why:** `start`/`finish` communicate the review session lifecycle better than git jargon.

**Changes:**

- `crates/git-forge/src/cli.rs`: Rename `ReviewCommand::Checkout` to `ReviewCommand::Start`.
  Rename `ReviewCommand::Done` to `ReviewCommand::Finish`.
- `crates/git-forge/src/exe.rs`: Update the match arms.
- Keep `checkout` and `done` as hidden aliases for backward compatibility (clap `#[command(alias = "checkout")]`).

---

### 4. Open editor from `forge review start`

**Why:** After creating the worktree, the developer should land in their editor immediately.

**Current behavior:** Spawns a subshell `cd`'d into the worktree.

**New behavior:** Detect and launch the editor.

**Changes:**

- `crates/git-forge/src/exe.rs`, the `ReviewCommand::Start` (née `Checkout`) arm:
  - Detection order: `$FORGE_EDITOR`, `$VISUAL`, `$EDITOR`, then try `zed`, `code`, `nvim`, `vim` in PATH.
  - For GUI editors (`zed`, `code`): exec `<editor> <worktree_path>` and return immediately.
  - For TUI editors (`nvim`, `vim`): retain the current subshell behavior (or exec directly).
  - Add `--no-editor` flag to skip this and just print the path.
- Keep the `--json` path unchanged (no editor launch, just output).

---

### 5. Add `forge.review.open` LSP command with `showDocument`

**Why:** Opens all files with unresolved comments in the editor, solving the discovery problem without custom UI.

**Changes:**

- `crates/forge-lsp/src/main.rs`:
  - Add a new `forge.review.open` command to `execute_command()`.
  - Implementation: walk all forge comment refs, collect files with unresolved threads, resolve blob OIDs to file paths via `head_blob_map()`, call `self.client.show_document()` for each file URI.
  - Register `forge.review.open` in the `ExecuteCommandOptions` capability.
  - Add a code action (visible on any line in any file) labeled "Open all files with unresolved comments" that triggers this command.
    This is the entry point until Zed exposes `workspace/executeCommand` in the command palette.

**Validation:** Open a project with forge comments in Zed.
Trigger the code action.
All commented files should open.
Diagnostics should appear on each.

**Risk:** Zed may not implement `window/showDocument`.
If it doesn't, fall back to logging the file list and inform the user.
File an upstream issue.

---

### 6. Expand language registration in Zed extension

**Changes:**

- `extensions/forge-zed/extension.toml`: Expand the `languages` list to cover common file types.
  At minimum:

  ```toml
  languages = [
    "Rust", "Python", "TypeScript", "JavaScript", "Go", "C", "C++",
    "Java", "Ruby", "Shell", "TOML", "YAML", "JSON", "Markdown",
    "CSS", "HTML", "Swift", "Kotlin", "Zig", "Elixir", "Haskell"
  ]
  ```

- Check if Zed supports a wildcard or `"*"` — if so, use that instead.

---

### 7. Add `forge review` bare command (no subcommand) as context-aware summary

**Why:** When standing in a review worktree, `forge review` with no arguments should show the review status — unresolved comment count, files with comments, approval state.
This is the "where am I, what's left" command.

**Changes:**

- `crates/git-forge/src/cli.rs`: Make the `ReviewCommand` subcommand optional (clap `#[command(subcommand_required = false)]` or add a default variant).
- `crates/git-forge/src/exe.rs`: When no subcommand is given:
  1. Detect active review from worktree context (`.git/forge-review` marker).
  2. List unresolved comment threads grouped by file.
  3. Show approval status.
  4. Print file paths relative to worktree root so output is useful for scripting.

---

### 8. Sync forge refs via forge-server (push, pull, merge conflicts)

**Why:** Backup and multi-machine access.
Review and issue artifacts should be preserved on the remote.
The forge CLI never pushes — forge-server owns all network I/O.

**Changes:**

- `crates/forge-server`: Before importing/exporting, fetch remote forge refs and resolve any conflicts.
  After importing/exporting, push `refs/forge/*` to the configured remote.
- Handle merge conflicts on forge refs gracefully — log a warning and preserve both sides (or pick the newer timestamp), rather than failing the entire sync.
- If push/pull fails (no remote, no network), warn but don't error — local-first means offline is always valid.

---

## Out of Scope for MVP

- GitHub review sync (kept disabled; issue sync only)
- Diff-based review comments (blob-anchored model is sufficient)
- Custom Zed panels or views (blocked on Zed extension API)
- CodeLens (not yet available in Zed; diagnostics + code actions are the MVP surface)
- TUI review browser (ratatouille `forge review` interactive mode — nice-to-have, not MVP)
- forge-nvim updates (Neovim extension can follow the same LSP changes but is not demo-critical)

## Ordering

Work items are ordered by dependency and demo-criticality:

1. **Item 6** (language registration) — trivial, unblocks everything else
2. **Item 5** (showDocument) — validate Zed support early; if it fails, adjust the demo flow
3. **Item 2** (worktree path) — standalone change, no dependencies
4. **Item 3** (rename start/finish) — pairs with item 2
5. **Item 4** (editor launch) — depends on item 2/3
6. **Item 7** (bare `forge review`) — depends on context detection working
7. **Item 1** (config-driven sync scope) — independent, can be done anytime
8. **Item 8** (server push/pull of forge refs) — pairs with item 1, depends on server sync being solid
