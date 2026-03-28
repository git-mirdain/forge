# Issues, Reviews, and Comments: Development Plan

## Context

The forge workspace was gutted (commit `93be628`) for a fresh start using git-data primitives.
Currently only two stub crates exist: `git-forge` (empty lib + empty binary) and `forge-mcp` (MCP server scaffold, no tools).
The git-data workspace provides three published crates — `git-ledger` (versioned records as refs), `git-chain` (append-only event streams), and `git-metadata` (annotations and relations) — that replace the boilerplate the old forge crates reimplemented.

**Goal**: Build issues, reviews, and comments in `git-forge` as a library backed by `git-ledger` and `git-chain`, then expose read tools via `forge-mcp` and write commands via the `forge` CLI.
The `forge-github` crate provides bidirectional sync with GitHub.
A `forge-server` daemon coordinates GitHub sync automatically; `forge sync` is a git-only manual fallback for pushing/fetching entity refs.

## Ref Layout

```text
refs/forge/issue/<oid>                 # entity ref, keyed by initial commit OID
refs/forge/review/<oid>                # entity ref, keyed by initial commit OID
refs/forge/comments/issue/<oid>        # chain, per-entity
refs/forge/comments/review/<oid>       # chain, per-entity
refs/forge/meta/index/issues           # display ID ↔ OID mapping
refs/forge/meta/index/reviews          # display ID ↔ OID mapping
refs/forge/config                      # contributors, entity registrations, sync config
refs/forge/sync/github/<owner>/<repo>  # GitHub bidirectional sync state (local-only)
```

Entity refs are keyed by the OID of the initial commit on that ref.
This OID is permanent — it never changes even as the ref tip advances with edits.
No UUIDs.

### Index

The index ref maps display IDs to OIDs and vice versa:

```text
refs/forge/meta/index/issues → commit → tree
  3         → blob "ab3f1c9e..."       # display ID → OID (local)
  ab3f1c9e  → blob "3"                # OID → display ID
  ff02c817  → blob "pending"          # staged, not yet synced
  auth-bug  → blob "3"                # user alias → display ID
  GH1       → blob "cc91d4f2..."      # GitHub issue #1 → OID (sigil-prefixed)
  cc91d4f2  → blob "GH1"             # OID → display ID (GitHub-namespaced)
```

Display IDs are strings: pure-numeric for local entities (`"3"`), sigil-prefixed for remote-sourced entities (`"GH1"`).
The sigil is configurable (see Phase 1 Step 1.6); `"GH"` is the default for GitHub.

### Resolution

Users reference entities with the `#` sigil.
The input after `#` is resolved through the index:

1. All digits → display ID lookup (e.g. `#3`).
2. Otherwise → OID prefix or alias lookup (e.g. `#ab3f`, `#auth-bug`, `#GH1`).
3. OID prefixes work like git SHAs — shortest unambiguous prefix accepted.

Both staged and synced entities resolve through the same mechanism.
GitHub-imported entities resolve via the sigil-prefixed display ID (e.g. `#GH1`).

### Entity Creation

Creation always writes a local entity ref immediately.
Display ID assignment is deferred to sync.

```text
$ forge issue new "Fix auth bug"
Created issue #ab3f1c9 (pending sync)

$ forge issue show #ab3f
# works immediately — indexed at creation time

$ forge sync
#ab3f1c9 → #3

$ forge issue show #3     # works
$ forge issue show #ab3f  # still works
```

### Sync and ID Assignment

`forge sync` is a git-only operation — it pushes/fetches entity refs and index refs.
It never talks to the GitHub API.

**Remote exists:**

1. Push entity refs to remote.
2. Server (trusted committer) assigns display IDs and writes the index.
3. Client fetches the updated index.

**No remote (air-gapped / local-only):**

1. Client assigns display IDs locally and writes the index itself.
2. If a remote is added later, the first sync pushes everything.
   Server may remap display IDs.

Display IDs are convenient but unstable until synced.
OID references are always stable.

### Write Protection

| Ref | Who writes | Protected? |
|---|---|---|
| `refs/forge/issue/<oid>` | Client (author) | Append-only per entity |
| `refs/forge/meta/index/*` | Server only (or client when no remote) | Yes |
| `refs/forge/config` | Server/admin only | Yes |

## Entity Tree Schemas

### Issue

```text
refs/forge/issue/<oid> → commit → tree
├── title           # plain text blob
├── state           # plain text: "open" or "closed"
├── body            # markdown blob
├── labels/         # directory: empty blobs whose names are labels
└── assignees/      # directory: empty blobs whose names are contributor IDs
```

No `author` blob — the first commit's author (from the git commit object) is the issue creator.
No `created` or `updated` blobs — use the commit's author timestamp.

### Review

```text
refs/forge/review/<oid> → commit → tree
├── meta/
│   ├── state           # "open", "merged", or "closed"
│   ├── ref             # optional UTF-8 blob: source ref name (e.g. "feature-branch")
│   └── target/
│       ├── base        # OID blob (optional — absent for single-object reviews)
│       └── head        # OID blob (required)
├── objects/            # pinned git objects to prevent GC
│   ├── <oid>           # blob (mode 100644), tree (mode 040000), or commit (mode 160000)
│   └── ...
├── title               # plain text
└── description         # markdown
```

No `author` blob — the first commit's author (from the git commit object) is the review creator.
No `created` or `updated` blobs — use the commit's author timestamp.
No revisions tree — revision tracking is out of scope for now.

**Target types.**
Reviews can target a blob, a tree (including a synthetic tree of blobs), a commit, or a commit range.
Object types are resolved dynamically via `git2::Object::kind()` (libgit2 `git_object_type`), not stored.

- Single object: only `meta/target/head` is present.
- Commit range: both `meta/target/base` and `meta/target/head` are present,
  using standard gitrevisions `base..head` semantics.
- Ref sugar: `forge review new main..feature` resolves the ref to a commit range at creation time and stores the ref name in `meta/ref`.
  When `meta/ref` is present, `forge sync` (or an explicit refresh) re-resolves the ref and updates `meta/target/*` and `objects/` accordingly.

**GC protection.**
The `objects/` tree stores the actual reviewed git objects as tree entries, keeping them reachable from the review ref and safe from garbage collection.
All three object types are supported natively via git tree entry modes: blobs (`100644`), trees (`040000`), and commits/gitlinks (`160000`).
The `git2` crate's `TreeBuilder::insert` and `FileMode` enum support all three modes.
When a review target is updated (e.g. after a rebase), the new objects are added to `objects/`; old objects remain reachable in earlier commits on the review ref (append-only).

**Comment re-anchoring after rebase.**
When a review's target is updated, comments anchored to the old target are not moved or deleted.
They remain reachable from earlier review commits.
A blame-style following algorithm re-anchors comments to the new target: for each anchored comment, read the blob at the original anchor, trace each line through `git blame` on the new target, and map to the equivalent line in the new revision.
When following fails (deleted lines, heavy rewrites), the comment is left anchored to its original object — a future server or product can surface these as "outdated" in the same way GitHub does.

## Approach

Free functions taking `&git2::Repository`, no traits or entity abstraction (per forge-abstractions.md "Do Not Extract Yet").
Issues and reviews delegate to `git-ledger::Ledger`.
Comments delegate to `git-chain::Chain`.

Development follows vertical slices: each entity type is built end-to-end (library → tests → MCP → CLI → GitHub sync → server) before starting the next.
The `forge` CLI binary lives in `git-forge` and never calls `forge-github` directly; only `forge-server` depends on both.

---

## Phase 0 — Dependencies

### Step 0.1: Workspace deps

**File**: `Cargo.toml` (root)

Add to `[workspace.dependencies]`:

```toml
git2 = "0.20.4"
git-ledger = "0.1.0-alpha.2"
git-chain = "0.1.0-alpha.1"
tempfile = "3"
octocrab = "0.44"
tokio = { version = "1", features = ["full"] }
toml = "0.8"
forge-github = { path = "crates/forge-github" }
forge-server = { path = "crates/forge-server" }
```

Add `"crates/forge-github"` and `"crates/forge-server"` to `[workspace] members`.

### Step 0.2: git-forge deps

**File**: `crates/git-forge/Cargo.toml`

```toml
[dependencies]
git2 = { workspace = true }
git-ledger = { workspace = true }
git-chain = { workspace = true }
anyhow = { workspace = true }
figue = { workspace = true }
serde = { workspace = true }

[dev-dependencies]
tempfile = { workspace = true }
```

### Step 0.3: forge-github crate scaffold

**Directory**: `crates/forge-github/`

**File**: `crates/forge-github/Cargo.toml`

```toml
[package]
name = "forge-github"
version = "0.0.0"
edition.workspace = true
publish.workspace = true
license.workspace = true
description = "GitHub bidirectional sync adapter for the forge store."

[dependencies]
git-forge = { workspace = true }
git2 = { workspace = true }
octocrab = { workspace = true }
tokio = { workspace = true }
anyhow = { workspace = true }
serde = { workspace = true }
toml = { workspace = true }

[dev-dependencies]
tempfile = { workspace = true }
```

**File**: `crates/forge-github/src/lib.rs` — empty stub:

```rust
//! GitHub bidirectional sync adapter for the forge store.
```

### Step 0.4: forge-server crate scaffold

**Directory**: `crates/forge-server/`

**File**: `crates/forge-server/Cargo.toml`

```toml
[package]
name = "forge-server"
version = "0.0.0"
edition.workspace = true
publish.workspace = true
license.workspace = true
description = "Forge sync daemon — watches refs and coordinates GitHub sync."

[[bin]]
name = "forge-server"
path = "src/main.rs"

[dependencies]
git-forge = { workspace = true }
forge-github = { workspace = true }
git2 = { workspace = true }
tokio = { workspace = true }
anyhow = { workspace = true }

[dev-dependencies]
tempfile = { workspace = true }
```

**File**: `crates/forge-server/src/main.rs` — minimal stub:

```rust
#[tokio::main]
async fn main() -> anyhow::Result<()> {
    println!("forge-server: not yet implemented");
    Ok(())
}
```

### Step 0.5: forge-mcp deps

**File**: `crates/forge-mcp/Cargo.toml`

Add `git2 = { workspace = true }` to `[dependencies]`.

**Verify**: `cargo check --workspace`

---

## Phase 1 — Issues

Full vertical slice: library → tests → MCP tools → CLI → GitHub import/export → server sync.

### Step 1.1: Module structure

**File**: `crates/git-forge/src/lib.rs`

```rust
//! Local-first infrastructure for Git forges.

pub mod issue;
pub mod refs;
```

**File**: `crates/git-forge/src/refs.rs` — ref prefix constants:

- `ISSUE_PREFIX = "refs/forge/issue/"`
- `REVIEW_PREFIX = "refs/forge/review/"`
- `ISSUE_COMMENTS_PREFIX = "refs/forge/comments/issue/"`
- `REVIEW_COMMENTS_PREFIX = "refs/forge/comments/review/"`
- `ISSUE_INDEX = "refs/forge/meta/index/issues"`
- `REVIEW_INDEX = "refs/forge/meta/index/reviews"`

### Step 1.2: Issue types and CRUD

**File**: `crates/git-forge/src/issue.rs`

**Types**:

- `IssueState { Open, Closed }` — derives `Serialize`
- `Issue { oid: String, display_id: Option<String>, title, state, body, labels: Vec<String>, assignees: Vec<String> }` — derives `Serialize`

The `oid` is the initial commit OID (permanent identity). `display_id` is `None` while staged, `Some("3")` for local, `Some("GH1")` for GitHub-synced.

**Field mapping** (LedgerEntry fields ↔ Issue):

| LedgerEntry field | Issue field | Notes |
|---|---|---|
| `"title"` | `title` | UTF-8 blob |
| `"state"` | `state` | `"open"` or `"closed"` |
| `"body"` | `body` | UTF-8 blob |
| `"labels/<name>"` | `labels` | Empty blob, name is the label |
| `"assignees/<name>"` | `assignees` | Empty blob, name is contributor ID |

**Functions** (all take `&git2::Repository`):

- `issue_from_entry(entry: &LedgerEntry) -> Result<Issue>` — parse fields
- `create_issue(repo, title, body, labels, assignees) -> Result<Issue>` — creates entity ref keyed by initial commit OID, writes OID → "pending" to index
- `get_issue(repo, oid_or_id) -> Result<Issue>` — resolve via index, then read
- `list_issues(repo) -> Result<Vec<Issue>>` — enumerate entity refs, resolve display IDs from index
- `list_issues_by_state(repo, state) -> Result<Vec<Issue>>` — filter after list
- `update_issue(repo, oid_or_id, title?, body?, state?, add_labels, remove_labels, add_assignees, remove_assignees) -> Result<Issue>` — resolve, build `Vec<Mutation>`, call `Ledger::update`

**Verify**: `cargo check -p git-forge`

### Step 1.3: Test infrastructure and issue tests

**File**: `crates/git-forge/src/lib.rs` — add `#[cfg(test)] mod tests;`

**File**: `crates/git-forge/src/tests.rs` — declare submodules:

```rust
mod issue;
```

**Shared helper** (in `tests.rs` or a `tests/helpers.rs`):

```rust
fn test_repo() -> (TempDir, Repository) {
    let dir = TempDir::new().unwrap();
    let repo = Repository::init(dir.path()).unwrap();
    let mut cfg = repo.config().unwrap();
    cfg.set_str("user.name", "Test").unwrap();
    cfg.set_str("user.email", "test@test.com").unwrap();
    drop(cfg);
    let sig = repo.signature().unwrap();
    let tree = repo.find_tree(repo.treebuilder(None).unwrap().write().unwrap()).unwrap();
    repo.commit(Some("HEAD"), &sig, &sig, "init", &tree, &[]).unwrap();
    (dir, repo)
}
```

**Issue tests** (`tests/issue.rs`):

- `create_returns_oid`
- `get_issue_by_oid_roundtrip`
- `list_issues`
- `list_issues_by_state`
- `update_title`
- `update_state`
- `labels_roundtrip`
- `assignees_roundtrip`
- `sync_assigns_display_id`
- `resolve_by_oid_prefix`

**Verify**: `cargo test -p git-forge`

### Step 1.4: MCP read tools — issues

**File**: `crates/forge-mcp/src/server.rs`

Add `repo_path: PathBuf` to `ForgeServer`.
Discover via `git2::Repository::discover(".")` in `new()`.
Add `fn open_repo(&self) -> anyhow::Result<Repository>`.

Add to `#[tool_router] impl ForgeServer`:

- `list_issues(state: Option<String>) -> String` — JSON array
- `get_issue(ref: String) -> String` — accepts display ID or OID prefix

Each opens repo, calls git-forge library, serializes via `serde_json::to_string_pretty`.

**Verify**: `cargo check -p forge-mcp`

### Step 1.5: CLI — issue commands

**File**: `crates/git-forge/src/main.rs`

```text
forge issue {new, show, list, edit, close, reopen, label, assign}
forge sync
```

Each command opens `Repository::discover(".")` and delegates to library functions.
Output to stdout.

`forge sync` is git-only: pushes entity refs to the forge remote and fetches the updated index.
It does not talk to the GitHub API.

Add `pub mod cli;` to `lib.rs` with the figue command definitions in `src/cli.rs`.
Figue's dynamic dispatch model allows future plugins (e.g. `forge-github` registering `forge github` subcommands) without compile-time coupling.

**Verify**: `cargo build -p git-forge` and `./target/debug/forge --help`

### Step 1.6: forge-github — config, sync state, and client

Shared infrastructure for both import and export directions.

**File**: `crates/forge-github/src/config.rs`

**Types**:

- `GitHubSyncConfig { owner: String, repo: String, sigil: String, token: Option<String> }`
  - `sigil` defaults to `"GH"` when absent from `refs/forge/config`
  - `token` falls back to `GITHUB_TOKEN` env var if `None`

**`refs/forge/config` tree layout** — tree of trees, each value is a plain UTF-8 blob:

```text
refs/forge/config → commit → tree
  sync/
    github/
      <owner>/
        <repo>/
          sigil   → blob "GH"
```

**Functions**:

- `read_github_config(repo: &Repository, owner: &str, repo_name: &str) -> Result<GitHubSyncConfig>` — navigates `sync/github/<owner>/<repo_name>/sigil` blob in `refs/forge/config` tree, fills defaults if absent
- `write_github_config(repo: &Repository, cfg: &GitHubSyncConfig) -> Result<()>` — writes blobs back into the tree, creates new commit on `refs/forge/config`

---

**File**: `crates/forge-github/src/state.rs`

Tracks which GitHub entities have been synced (both directions).

```text
refs/forge/sync/github/<owner>/<repo> → commit → tree
  issues/<github_number>              → blob "<forge_oid>"
  reviews/<github_number>             → blob "<forge_oid>"
  comments/<github_comment_id>        → blob "<chain_entry_oid>"
```

`<github_number>` is the decimal string of the GitHub issue/PR number.
This ref is written locally; it is never pushed to the forge remote.

Both import and export write to the same state:

- **Import** writes `issues/<n> → <oid>` after creating a forge entity from a GitHub issue.
- **Export** writes `issues/<n> → <oid>` after creating a GitHub issue from a forge entity.
- Either direction can check whether a mapping already exists to avoid duplicates.

**Functions**:

- `sync_ref_name(owner: &str, repo: &str) -> String` — `"refs/forge/sync/github/<owner>/<repo>"`
- `load_sync_state(repo: &Repository, owner: &str, repo_name: &str) -> Result<HashMap<String, String>>` — returns `"issues/<n>"` → OID map
- `save_sync_state(repo: &Repository, owner: &str, repo_name: &str, state: &HashMap<String, String>) -> Result<()>` — writes new commit to sync ref
- `lookup_by_github_id(state: &HashMap<String, String>, kind: &str, github_number: u64) -> Option<&str>` — key is `"issues/<n>"` or `"reviews/<n>"`
- `lookup_by_forge_oid(state: &HashMap<String, String>, kind: &str, forge_oid: &str) -> Option<u64>` — reverse scan of `<kind>/*` values for matching OID; returns the GitHub number

---

**File**: `crates/forge-github/src/client.rs`

Thin wrapper over `octocrab` to page through all results.

**Types** (serde structs mirroring the GitHub REST API response fields used):

- `GhIssue { number: u64, title: String, body: Option<String>, state: String, labels: Vec<GhLabel>, assignees: Vec<GhUser>, user: GhUser, created_at: String, pull_request: Option<()> }`
  - `pull_request` field presence distinguishes PRs from issues in the list API
- `GhIssueComment { id: u64, body: Option<String>, user: GhUser, created_at: String }`
- `GhLabel { name: String }`
- `GhUser { login: String }`

Pull request types (`GhPull`, `GhReviewComment`, `GhRef`) are added in Phase 3.

**Functions** (all `async`):

- `make_client(token: Option<&str>) -> Result<Octocrab>` — builds octocrab instance; if token is None, checks `GITHUB_TOKEN` env var; returns error if neither present
- `fetch_issues(client: &Octocrab, owner: &str, repo: &str) -> Result<Vec<GhIssue>>` — pages through `/repos/{owner}/{repo}/issues?state=all&filter=all`, excludes entries where `pull_request` is present
- `fetch_issue_comments(client: &Octocrab, owner: &str, repo: &str, number: u64) -> Result<Vec<GhIssueComment>>`
- `create_github_issue(client: &Octocrab, owner: &str, repo: &str, title: &str, body: &str, labels: &[String], assignees: &[String]) -> Result<u64>` — POST, returns issue number
- `update_github_issue(client: &Octocrab, owner: &str, repo: &str, number: u64, title: Option<&str>, body: Option<&str>, state: Option<&str>, labels: Option<&[String]>, assignees: Option<&[String]>) -> Result<()>` — PATCH
- `create_github_issue_comment(client: &Octocrab, owner: &str, repo: &str, number: u64, body: &str) -> Result<u64>` — POST, returns comment ID

**Verify**: `cargo check -p forge-github`

### Step 1.7: GitHub import — issues

**File**: `crates/forge-github/src/import.rs`

**Types**:

- `SyncReport { imported: usize, exported: usize, skipped: usize, failed: usize }`

**Mapping** (GitHub → forge):

| GitHub field | Forge field | Notes |
|---|---|---|
| `title` | `title` | direct |
| `body` | `body` | `None` → empty string |
| `state` | `state` | map directly |
| `labels[].name` | `labels` | direct |
| `assignees[].login` | `assignees` | GitHub login as contributor ID |
| `user.login` | commit author name | used as git signature name in ledger commit |
| `created_at` | commit author timestamp | RFC 3339 → git time |
| issue `number` | display ID alias | stored as `"<sigil><number>"`, e.g. `"GH#1"` (default sigil is `"GH#"`) |

For the git signature on imported commits, use `(user.login, "<login>@users.noreply.github.com")` so authorship is preserved without exposing real emails.
The commit timestamp should use `created_at` from the GitHub payload, not the current time.

**Functions**:

- `import_issues(repo: &Repository, cfg: &GitHubSyncConfig) -> Result<SyncReport>` — async
  1. Build octocrab client from `cfg.token`
  2. Load sync state
  3. Fetch all issues (excluding PRs)
  4. For each: if `"issues/<n>"` in sync state → skip; else call `git_forge::issue::create_issue`, write display ID `"<sigil><n>"` to index immediately (no "pending"), update sync state
  5. Save sync state

### Step 1.8: GitHub export — issues

**File**: `crates/forge-github/src/export.rs`

**Functions**:

- `export_issue(repo: &Repository, cfg: &GitHubSyncConfig, forge_oid: &str) -> Result<u64>` — async
  1. Check sync state: if `lookup_by_forge_oid(state, "issues", forge_oid)` returns `Some` → already exported, return the GitHub number
  2. Read issue from `refs/forge/issue/<forge_oid>`
  3. Call `create_github_issue(client, owner, repo, title, body, labels, assignees)`
  4. Get back GitHub issue number `n`
  5. Write `issues/<n> → <forge_oid>` to sync state
  6. Write `<sigil><n> → <forge_oid>` and `<forge_oid> → <sigil><n>` aliases to index
  7. Return `n`

- `export_pending_issues(repo: &Repository, cfg: &GitHubSyncConfig) -> Result<SyncReport>` — async
  1. Scan index for entries with value `"pending"`
  2. For each pending OID: call `export_issue`
  3. On success, the pending entry is replaced by the `<sigil><n>` alias

- `sync_issue_updates(repo: &Repository, cfg: &GitHubSyncConfig) -> Result<SyncReport>` — async
  1. For each entity in sync state: compare forge entity tip commit with a stored "last synced" marker
  2. If the forge entity has been updated since last sync: call `update_github_issue` with changed fields
  3. Future: also detect GitHub-side updates and pull them into the forge entity

### Step 1.9: forge-server — issue sync

**File**: `crates/forge-server/src/main.rs`

The forge-server daemon watches for new forge entities and coordinates GitHub sync.

**Startup behavior**:

1. Open `Repository::discover(".")`
2. Read GitHub config from `refs/forge/config`; if absent, run without GitHub sync
3. Assign display IDs for any pending entities (trusted committer role)
4. Run initial sync: `import_issues` then `export_pending_issues`
5. Enter poll loop

**Poll loop** (configurable interval, default 60s):

1. Scan for new entity refs (pending entries in index)
2. Export pending entities to GitHub
3. Import new GitHub issues
4. Assign display IDs for any remaining pending entities

**CLI**:

```text
forge-server [--repo <path>] [--poll-interval <seconds>]
forge-server --once   # single sync pass, then exit
```

`--once` is the manual fallback for GitHub sync when the daemon isn't running long-term.

**Verify**: `cargo build -p forge-server`

### Step 1.10: forge-github tests — issues

**File**: `crates/forge-github/src/tests/`

Tests use a `TempDir` git repo (same `test_repo()` helper pattern) and mock GitHub API responses — no live network calls.
Use `octocrab`'s mock support or pass client functions as function pointers.

- `import_single_issue_creates_ref` — verify `refs/forge/issue/<oid>` exists and display ID `"GH1"` resolves
- `import_skips_already_imported` — run import twice, second run produces `skipped = 1`
- `sigil_configurable` — set `sigil = "ACME"`, verify display ID is `"ACME1"`
- `export_issue_creates_github_issue` — verify sync state has `issues/<n> → <oid>` and index has `GH<n>` alias
- `export_skips_already_exported` — export same OID twice, second call is a no-op
- `roundtrip_no_duplicates` — export an issue, then run import; imported set is empty (sync state prevents re-import)

**Verify**: `cargo test -p forge-github`

---

## Phase 2 — Comments

Adds comment support for issues.
Comments use `git-chain` for append-only event streams.
The `forge` CLI gets comment commands; the server extends to sync issue comments with GitHub.

### Step 2.1: Comment types and operations

**File**: `crates/git-forge/src/comment.rs`

Add `pub mod comment;` to `lib.rs`.

**Types**:

- `Anchor` enum: `Object { oid, range: Option<String> }`, `CommitRange { start, end }` — derives `Serialize`
- `Comment { oid, body, author_name, author_email, timestamp: i64, anchor: Option<Anchor>, resolved: bool, replaces: Option<String>, reply_to: Option<String>, tree: String }` — derives `Serialize`

**Trailer format** (from git-forge-comments.md):

| Trailer | Purpose |
|---|---|
| `Anchor: <sha>` | target object |
| `Anchor-Range: 42-47` | line range (blob only) |
| `Anchor-End: <sha>` | end of commit range |
| `Resolved: true` | marks thread resolved |
| `Replaces: <oid>` | edit marker |

**Helpers**:

- `format_trailers(anchor: Option<&Anchor>, resolved: bool, replaces: Option<&str>) -> String`
- `parse_trailers(message: &str) -> (String, HashMap<String, String>)` — returns (body, trailers).
  Trailer block = last paragraph where every non-empty line matches `^[\w-]+: .+$`
- `comment_from_chain_entry(repo: &Repository, entry: &ChainEntry) -> Result<Comment>` — load commit, parse message, extract second parent
- `issue_comment_ref(oid: &str) -> String`
- `review_comment_ref(oid: &str) -> String`

**Functions**:

- `add_comment(repo, ref_name, body, anchor?) -> Result<Comment>` — format message with trailers, `repo.build_tree(&[])` for empty tree, `Chain::append(ref_name, msg, tree, None)`
- `add_reply(repo, ref_name, body, reply_to_oid, anchor?) -> Result<Comment>` — same but `Chain::append(..., Some(reply_to_oid))`
- `resolve_comment(repo, ref_name, reply_to_oid, message?) -> Result<Comment>` — append with `Resolved: true` trailer
- `edit_comment(repo, ref_name, original_oid, new_body, anchor?) -> Result<Comment>` — append with `Replaces: <oid>` trailer, second parent = original
- `list_comments(repo, ref_name) -> Result<Vec<Comment>>` — `Chain::walk(ref_name, None)`, parse each
- `list_thread(repo, ref_name, root_oid) -> Result<Vec<Comment>>` — `Chain::walk(ref_name, Some(root_oid))`

**Note on empty tree**: `Chain::build_tree(&[])` should produce the empty tree.
If it doesn't, use the constant `4b825dc642cb6eb9a060e54bf899d15b4fdd19d0` parsed as `Oid`.

**Verify**: `cargo check -p git-forge`

### Step 2.2: Comment tests

**File**: `crates/git-forge/src/tests.rs` — add `mod comment;`

**Comment tests** (`tests/comment.rs`):

- `add_comment_creates_chain`
- `add_second_comment`
- `reply_threading`
- `list_comments_chronological`
- `list_thread`
- `resolve_sets_trailer`
- `edit_sets_replaces`
- `anchor_object_with_range`
- `anchor_commit_range`

**Verify**: `cargo test -p git-forge`

### Step 2.3: MCP read tools — issue comments

**File**: `crates/forge-mcp/src/server.rs`

Add to `#[tool_router] impl ForgeServer`:

- `list_issue_comments(ref: String) -> String` — accepts issue display ID or OID prefix

**Verify**: `cargo check -p forge-mcp`

### Step 2.4: CLI — comment commands

**File**: `crates/git-forge/src/main.rs`

```text
forge comment {add, reply, resolve, list}
```

Commands take an `--issue <ref>` flag to specify the parent entity.
Phase 3 extends these with `--review <ref>`.

**Verify**: `cargo build -p git-forge`

### Step 2.5: GitHub import — issue comments

**File**: `crates/forge-github/src/import.rs`

- `import_issue_comments(repo: &Repository, cfg: &GitHubSyncConfig, github_number: u64) -> Result<SyncReport>` — async
  1. Fetch issue comments from GitHub API
  2. For each comment: check sync state `comments/<github_comment_id>`; if present → skip
  3. Call `git_forge::comment::add_comment` with body text
  4. Add `Github-Id: <id>` trailer to the chain commit message for secondary dedup
  5. Write `comments/<github_comment_id> → <chain_entry_oid>` to sync state

Update `import_issues` to call `import_issue_comments` for each imported issue.

### Step 2.6: GitHub export — issue comments

**File**: `crates/forge-github/src/export.rs`

- `export_issue_comments(repo: &Repository, cfg: &GitHubSyncConfig, forge_issue_oid: &str) -> Result<SyncReport>` — async
  1. Look up GitHub issue number via `lookup_by_forge_oid(state, "issues", forge_issue_oid)`; if `None` → issue not yet exported, skip
  2. Walk the comment chain for the issue
  3. For each chain entry: check sync state `comments/*` values for matching chain entry OID; if present → skip
  4. Call `create_github_issue_comment(client, owner, repo, github_number, body)`
  5. Write `comments/<github_comment_id> → <chain_entry_oid>` to sync state

Update `export_pending_issues` to also call `export_issue_comments` for each exported issue.

### Step 2.7: forge-server — comment sync

**File**: `crates/forge-server/src/main.rs`

Extend the poll loop:

1. After issue import/export, sync comments for all synced issues
2. Import new GitHub comments → forge chains
3. Export new forge chain entries → GitHub comments

### Step 2.8: forge-github tests — comments

- `import_issue_comments_adds_chain` — import a GitHub issue's comments, verify chain entries
- `import_comment_skips_already_imported` — idempotent re-run
- `export_issue_comment_creates_github_comment` — verify sync state has `comments/<id> → <chain_oid>`
- `export_comment_skips_already_exported` — idempotent re-run
- `roundtrip_comments_no_duplicates` — export comments, then import; no duplicates

**Verify**: `cargo test -p forge-github`

---

## Phase 3 — Reviews

Full vertical slice for reviews, including review comments (which are structurally different from issue comments — they anchor to commits, files, and lines).

### Step 3.1: Review types and CRUD

**File**: `crates/git-forge/src/review.rs`

Add `pub mod review;` to `lib.rs`.

**Types**:

- `ReviewState { Open, Merged, Closed }` — derives `Serialize`
- `ReviewTarget { head: String, base: Option<String> }` — derives `Serialize`
- `Review { oid: String, display_id: Option<String>, title, target: ReviewTarget, source_ref: Option<String>, state, description }` — derives `Serialize`

Same as issues: `oid` is the initial commit OID, `display_id` is `None` while staged, `Some("GH1")` for GitHub-synced.
No timestamp fields — use the git commit's author date for created/updated.

**Field mapping** (LedgerEntry ↔ Review):

| LedgerEntry field | Review field | Format |
|---|---|---|
| `"meta/state"` | state | `"open"`, `"merged"`, or `"closed"` |
| `"meta/ref"` | source_ref | optional UTF-8 blob (ref name) |
| `"meta/target/head"` | target.head | OID string |
| `"meta/target/base"` | target.base | optional OID string |
| `"objects/<oid>"` | (not in struct) | pinned objects, managed internally |
| `"title"` | title | plain text |
| `"description"` | description | markdown |

**Functions**:

- `review_from_entry(entry: &LedgerEntry) -> Result<Review>`
- `create_review(repo, title, description, target: ReviewTarget, source_ref: Option<&str>) -> Result<Review>` — creates entity ref keyed by initial commit OID, pins target objects in `objects/`, writes OID → "pending" to index
- `get_review(repo, oid_or_id) -> Result<Review>`
- `list_reviews(repo) -> Result<Vec<Review>>`
- `list_reviews_by_state(repo, state) -> Result<Vec<Review>>`
- `update_review(repo, oid_or_id, title?, description?, state?) -> Result<Review>`
- `refresh_review_target(repo, oid_or_id) -> Result<Review>` — re-resolves `meta/ref` to update `meta/target/*` and `objects/`; no-op if `meta/ref` is absent

**Verify**: `cargo check -p git-forge`

### Step 3.2: Review tests

**File**: `crates/git-forge/src/tests.rs` — add `mod review;`

**Review tests** (`tests/review.rs`):

- `create_returns_oid`
- `create_stores_all_fields`
- `create_with_commit_range_target`
- `create_with_single_blob_target`
- `create_with_source_ref`
- `objects_tree_pins_target`
- `get_review_roundtrip`
- `list_reviews`
- `list_reviews_by_state`
- `update_title_and_description`
- `update_state_to_merged`
- `refresh_target_updates_objects`
- `refresh_noop_without_ref`

**Verify**: `cargo test -p git-forge`

### Step 3.3: MCP read tools — reviews and review comments

**File**: `crates/forge-mcp/src/server.rs`

Add to `#[tool_router] impl ForgeServer`:

- `list_reviews(state: Option<String>) -> String`
- `get_review(ref: String) -> String` — accepts display ID or OID prefix
- `list_review_comments(ref: String) -> String`

**Verify**: `cargo check -p forge-mcp`

### Step 3.4: CLI — review commands

**File**: `crates/git-forge/src/main.rs`

```text
forge review {new, show, list, edit, close, merge}
```

Extend `forge comment` commands to accept `--review <ref>` in addition to `--issue <ref>`.

**Verify**: `cargo build -p git-forge`

### Step 3.5: GitHub client — pull requests and review comments

**File**: `crates/forge-github/src/client.rs`

Add types:

- `GhPull { number: u64, title: String, body: Option<String>, state: String, merged: bool, base: GhRef, head: GhRef, user: GhUser, created_at: String }`
- `GhReviewComment { id: u64, body: Option<String>, user: GhUser, commit_id: String, path: Option<String>, line: Option<u32>, created_at: String }`
- `GhRef { ref_field: String }` — `#[serde(rename = "ref")]`

Add functions (all `async`):

- `fetch_pulls(client: &Octocrab, owner: &str, repo: &str) -> Result<Vec<GhPull>>` — pages through `/repos/{owner}/{repo}/pulls?state=all`
- `fetch_review_comments(client: &Octocrab, owner: &str, repo: &str, number: u64) -> Result<Vec<GhReviewComment>>`
- `create_github_pull(client: &Octocrab, owner: &str, repo: &str, title: &str, body: &str, head: &str, base: &str) -> Result<u64>` — POST, returns PR number
- `update_github_pull(client: &Octocrab, owner: &str, repo: &str, number: u64, title: Option<&str>, body: Option<&str>, state: Option<&str>) -> Result<()>` — PATCH
- `create_github_review_comment(client: &Octocrab, owner: &str, repo: &str, number: u64, body: &str, commit_id: &str, path: &str, line: u32) -> Result<u64>` — POST, returns comment ID

### Step 3.6: GitHub import — reviews and review comments

**File**: `crates/forge-github/src/import.rs`

**Additional mapping** (GitHub → forge, reviews):

| GitHub field | Forge field | Notes |
|---|---|---|
| `title` | `title` | direct |
| `body` | `description` | `None` → empty string |
| `state` (+ `merged`) | `state` | `merged=true` → `"merged"`, else map `state` |
| `user.login` | commit author name | git signature |
| `created_at` | commit author timestamp | RFC 3339 → git time |
| `base.ref` | `meta/ref` + `meta/target/head` | ref stored for refresh, HEAD commit resolved and pinned in `objects/` |
| PR `number` | display ID alias | `"<sigil><number>"` in review index |

For the git signature on imported commits, use `(user.login, "<login>@github.invalid")`.

**Functions**:

- `import_reviews(repo: &Repository, cfg: &GitHubSyncConfig) -> Result<SyncReport>` — async; same flow as `import_issues` using `fetch_pulls` and `git_forge::review::create_review`; maps `base.ref` → `source_ref`, resolves `head.sha` → `ReviewTarget { head, base: None }`; pins the head commit in `objects/`
- `import_review_comments(repo: &Repository, cfg: &GitHubSyncConfig, github_number: u64) -> Result<SyncReport>` — async; maps `GhReviewComment.commit_id` + `line` to `Anchor::Object { oid: commit_id, range: Some("<line>-<line>") }`, calls `add_comment`; writes `comments/<github_comment_id> → <chain_oid>` to sync state
- `import_all(repo: &Repository, cfg: &GitHubSyncConfig) -> Result<SyncReport>` — calls `import_issues` then `import_reviews`, then imports comments for each

**Github-Id trailer**: add `Github-Id: <id>` to comment commit messages so re-runs can detect already-imported comments without scanning the whole chain.

### Step 3.7: GitHub export — reviews and review comments

**File**: `crates/forge-github/src/export.rs`

- `export_review(repo: &Repository, cfg: &GitHubSyncConfig, forge_oid: &str) -> Result<u64>` — async
  1. Check sync state: if already exported → return GitHub number
  2. Read review from `refs/forge/review/<forge_oid>`
  3. Resolve `source_ref` to get head/base branch names for the PR
  4. Call `create_github_pull(client, owner, repo, title, body, head_ref, base_ref)`
  5. Write `reviews/<n> → <forge_oid>` to sync state
  6. Write `<sigil><n>` alias to review index
  7. Return `n`

- `export_pending_reviews(repo: &Repository, cfg: &GitHubSyncConfig) -> Result<SyncReport>` — async; scans review index for pending entries

- `export_review_comments(repo: &Repository, cfg: &GitHubSyncConfig, forge_review_oid: &str) -> Result<SyncReport>` — async; same pattern as issue comment export but uses `create_github_review_comment` with anchor data (commit, path, line)

- `export_all(repo: &Repository, cfg: &GitHubSyncConfig) -> Result<SyncReport>` — calls `export_pending_issues` + `export_pending_reviews` + comments for all synced entities

### Step 3.8: forge-server — full sync

**File**: `crates/forge-server/src/main.rs`

Extend the poll loop to cover reviews and review comments.
The full sync cycle is now:

1. Import: `import_all` (issues + reviews + all comments)
2. Export: `export_all` (pending issues + pending reviews + new comments)
3. Assign display IDs for remaining pending entities

### Step 3.9: forge-github tests — reviews

- `import_single_pr_creates_ref` — verify `refs/forge/review/<oid>` exists and `"GH1"` resolves in review index
- `import_issue_and_pr_no_collision` — issue `#GH1` and review `#GH1` coexist in their respective indexes
- `import_review_comments_adds_chain` — import PR comments with anchors, verify chain entries
- `pr_merged_state_maps_correctly` — `merged: true` → review state `"merged"`
- `export_review_creates_github_pr` — verify sync state and review index alias
- `export_review_comments` — verify sync state tracks comment IDs
- `roundtrip_reviews_no_duplicates` — export then import, no duplicates

**Verify**: `cargo test -p forge-github`

---

## Commit Strategy

One commit per step or logical unit within each phase:

1. `feat: add workspace dependencies` (Phase 0)
2. `feat: add issue CRUD backed by git-ledger` (Steps 1.1–1.2)
3. `test: add issue tests` (Step 1.3)
4. `feat: add MCP read tools for issues` (Step 1.4)
5. `feat: add CLI issue commands and forge sync` (Step 1.5)
6. `feat: add forge-github config, sync state, and client` (Step 1.6)
7. `feat: add GitHub import for issues` (Step 1.7)
8. `feat: add GitHub export for issues` (Step 1.8)
9. `feat: add forge-server with issue sync` (Step 1.9)
10. `test: add forge-github issue tests` (Step 1.10)
11. `feat: add comment operations backed by git-chain` (Steps 2.1–2.2)
12. `feat: add MCP and CLI for issue comments` (Steps 2.3–2.4)
13. `feat: add GitHub import/export for issue comments` (Steps 2.5–2.6)
14. `feat: extend forge-server with comment sync` (Steps 2.7–2.8)
15. `feat: add review CRUD backed by git-ledger` (Steps 3.1–3.2)
16. `feat: add MCP and CLI for reviews` (Steps 3.3–3.4)
17. `feat: add GitHub import/export for reviews and review comments` (Steps 3.5–3.7)
18. `feat: extend forge-server with full sync` (Steps 3.8–3.9)

## Verification

After all phases:

1. `cargo test --workspace` — all tests pass
2. `cargo clippy --workspace` — no warnings
3. Manual: create a test repo, `forge issue new "test"`, `forge issue list`, verify refs exist with `git for-each-ref refs/forge/`
4. Manual: `forge comment add --issue #1 "first comment"`, verify chain ref
5. Manual: start `forge-server`, create a new issue with `forge issue new`, verify it appears on GitHub within one poll cycle
6. Manual: create an issue on GitHub, wait for poll, verify `forge issue list` shows the `#GH<n>` alias
7. Manual: `forge-server --once` as a one-shot sync fallback
