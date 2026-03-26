# Issues, Reviews, and Comments: Development Plan

## Context

The forge workspace was gutted (commit `93be628`) for a fresh start using git-data primitives.
Currently only two stub crates exist: `git-forge` (empty lib + empty binary) and `forge-mcp` (MCP server scaffold, no tools).
The git-data workspace provides three published crates — `git-ledger` (versioned records as refs), `git-chain` (append-only event streams), and `git-metadata` (annotations and relations) — that replace the boilerplate the old forge crates reimplemented.

**Goal**: Build issues, reviews, and comments in `git-forge` as a library backed by `git-ledger` and `git-chain`, then expose read tools via `forge-mcp` and write commands via the `forge` CLI.

## Ref Layout (from git-forge.md)

```text
refs/forge/issue/<id>                  # ledger, sequential
refs/forge/review/<id>                 # ledger, sequential
refs/forge/comments/issue/<id>         # chain, per-entity
refs/forge/comments/review/<id>        # chain, per-entity
```

## Approach

Free functions taking `&git2::Repository`, no traits or entity abstraction (per forge-abstractions.md "Do Not Extract Yet").
Issues and reviews delegate to `git-ledger::Ledger`.
Comments delegate to `git-chain::Chain`.

---

## Phase 0 — Dependencies

### Step 0.1: Workspace deps

**File**: `Cargo.toml` (root)

Add to `[workspace.dependencies]`:

```toml
git2 = "0.20.4"
git-ledger = "0.1.0-alpha.1"
git-chain = "0.1.0-alpha.1"
tempfile = "3"
```

### Step 0.2: git-forge deps

**File**: `crates/git-forge/Cargo.toml`

```toml
[dependencies]
git2 = { workspace = true }
git-ledger = { workspace = true }
git-chain = { workspace = true }
anyhow = { workspace = true }
clap = { workspace = true }
serde = { workspace = true }

[dev-dependencies]
tempfile = { workspace = true }
```

### Step 0.3: forge-mcp deps

**File**: `crates/forge-mcp/Cargo.toml`

Add `git2 = { workspace = true }` to `[dependencies]`.

**Verify**: `cargo check --workspace`

---

## Phase 1 — Library: Issues

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

### Step 1.2: Issue types and CRUD

**File**: `crates/git-forge/src/issue.rs`

**Types**:

- `IssueState { Open, Closed }` — derives `Serialize`
- `Issue { id: u64, title, state, body, labels: Vec<String>, assignees: Vec<String> }` — derives `Serialize`

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
- `create_issue(repo, title, body, labels, assignees) -> Result<Issue>` — `Ledger::create` with `IdStrategy::Sequential`
- `get_issue(repo, id) -> Result<Issue>` — `Ledger::read` then parse
- `list_issues(repo) -> Result<Vec<Issue>>` — `Ledger::list` then read each
- `list_issues_by_state(repo, state) -> Result<Vec<Issue>>` — filter after list
- `update_issue(repo, id, title?, body?, state?, add_labels, remove_labels, add_assignees, remove_assignees) -> Result<Issue>` — build `Vec<Mutation>`, call `Ledger::update`

**Verify**: `cargo check -p git-forge`

---

## Phase 2 — Library: Reviews

### Step 2.1: Review types and CRUD

**File**: `crates/git-forge/src/review.rs`

Add `pub mod review;` to `lib.rs`.

**Types**:

- `ReviewState { Open, Merged, Closed }` — derives `Serialize`
- `Revision { index: String, head_commit: String, timestamp: String }` — derives `Serialize`
- `Review { id: u64, title, target_branch, state, created, description, revisions: Vec<Revision> }` — derives `Serialize`

**Field mapping** (LedgerEntry ↔ Review):

| LedgerEntry field | Review field | Format |
|---|---|---|
| `"meta"` | target_branch, state, created | key = value, one per line |
| `"title"` | title | plain text |
| `"description"` | description | markdown |
| `"revisions/001"` | revisions[0] | key = value: head_commit, timestamp |

**Helpers**:

- `format_kv(pairs: &[(&str, &str)]) -> Vec<u8>` — serialize `"key = value\n"` lines
- `parse_kv(content: &str) -> HashMap<&str, &str>` — parse `line.split_once(" = ")`
- `now_rfc3339() -> String` — format current time as RFC 3339 using only `std::time`

**Functions**:

- `review_from_entry(entry: &LedgerEntry) -> Result<Review>`
- `create_review(repo, title, description, target_branch, head_commit) -> Result<Review>` — author from `repo.signature()`, timestamp via `now_rfc3339()`
- `get_review(repo, id) -> Result<Review>`
- `list_reviews(repo) -> Result<Vec<Review>>`
- `list_reviews_by_state(repo, state) -> Result<Vec<Review>>`
- `update_review(repo, id, title?, description?, state?) -> Result<Review>` — for state change: parse meta, update state line, re-serialize, `Mutation::Set("meta", ...)`
- `add_revision(repo, id, head_commit) -> Result<Review>` — count existing `revisions/*`, next index zero-padded to 3 digits

**Verify**: `cargo check -p git-forge`

---

## Phase 3 — Library: Comments

### Step 3.1: Comment types and operations

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
- `issue_comment_ref(id: u64) -> String`
- `review_comment_ref(id: u64) -> String`

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

---

## Phase 4 — Tests

### Step 4.1: Test infrastructure

**File**: `crates/git-forge/src/lib.rs` — add `#[cfg(test)] mod tests;`

**File**: `crates/git-forge/src/tests.rs` — declare submodules:

```rust
mod issue;
mod review;
mod comment;
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

### Step 4.2: Issue tests (`tests/issue.rs`)

- `create_assigns_id_1`
- `create_sequential_ids`
- `get_issue_roundtrip`
- `list_issues_sorted`
- `list_issues_by_state`
- `update_title`
- `update_state`
- `labels_roundtrip`
- `assignees_roundtrip`

### Step 4.3: Review tests (`tests/review.rs`)

- `create_assigns_id_1`
- `create_stores_all_fields`
- `get_review_roundtrip`
- `list_reviews`
- `list_reviews_by_state`
- `update_title_and_description`
- `update_state_to_merged`
- `add_revision`
- `revision_indices_zero_padded`

### Step 4.4: Comment tests (`tests/comment.rs`)

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

---

## Phase 5 — MCP Server (Read Tools)

### Step 5.1: ForgeServer gets repo path

**File**: `crates/forge-mcp/src/server.rs`

Add `repo_path: PathBuf` to `ForgeServer`.
Discover via `git2::Repository::discover(".")` in `new()`.
Add `fn open_repo(&self) -> anyhow::Result<Repository>`.

### Step 5.2: Tool handlers

**File**: `crates/forge-mcp/src/server.rs`

Add to `#[tool_router] impl ForgeServer`:

- `list_issues(state: Option<String>) -> String` — JSON array
- `get_issue(id: u64) -> String` — JSON object
- `list_reviews(state: Option<String>) -> String`
- `get_review(id: u64) -> String`
- `list_issue_comments(id: u64) -> String`
- `list_review_comments(id: u64) -> String`

Each opens repo, calls git-forge library, serializes via `serde_json::to_string_pretty`.

**Verify**: `cargo check -p forge-mcp`

---

## Phase 6 — CLI

### Step 6.1: CLI structure

**File**: `crates/git-forge/src/main.rs`

```text
forge issue {new, show, list, edit, close, reopen, label, assign}
forge review {new, show, list, edit, close, merge, revise}
forge comment {add, reply, resolve, list}
```

Each command opens `Repository::discover(".")` and delegates to library functions.
Output to stdout.

Add `pub mod cli;` to `lib.rs` with the clap `Subcommand` enums in `src/cli.rs`.

**Verify**: `cargo build -p git-forge` and `./target/debug/forge --help`

---

## Commit Strategy

One commit per phase or logical unit:

1. `feat: add git-data dependencies` (Phase 0)
2. `feat: add issue CRUD backed by git-ledger` (Phase 1 + tests)
3. `feat: add review CRUD backed by git-ledger` (Phase 2 + tests)
4. `feat: add comment operations backed by git-chain` (Phase 3 + tests)
5. `feat: add MCP read tools for issues, reviews, and comments` (Phase 5)
6. `feat: add forge CLI for issues, reviews, and comments` (Phase 6)

## Verification

After all phases:

1. `cargo test --workspace` — all tests pass
2. `cargo clippy --workspace` — no warnings
3. Manual: create a test repo, `forge issue new "test"`, `forge issue list`, verify refs exist with `git for-each-ref refs/forge/`
