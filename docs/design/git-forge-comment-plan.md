# `git-forge-comment` Development Plan

Each step is self-contained and ordered so that later steps build on earlier ones.
Feed them sequentially to an agent.

---

## Step 1 — Wire up crate dependencies

**Context**: `crates/git-forge-comment/Cargo.toml` has no dependencies and `src/lib.rs` contains only a stub `Comment` struct that duplicates what already exists in `git-forge-core`.
The crate is not yet a member of the workspace `[dependencies]` table and is not referenced from `crates/git-forge`.

**Tasks**:

1. In `crates/git-forge-comment/Cargo.toml`, add:
   - `git2 = { workspace = true }`
   - `clap = { workspace = true }`
   - `tempfile` under `[dev-dependencies]`
   - `criterion` under `[dev-dependencies]` with a `[[bench]] name = "comments"` entry
2. Add `git-forge-comment = { path = "crates/git-forge-comment" }` to the
   workspace `[workspace.dependencies]` table in the root `Cargo.toml`.
3. Add `git-forge-comment` to `crates/git-forge/Cargo.toml` `[dependencies]`.
4. Delete the stub `Comment` struct from `crates/git-forge-comment/src/lib.rs` and
   replace the file with a proper crate-level doc comment and module declarations
   that match what the later steps will add (`pub mod cli`, `pub mod exe`,
   `pub mod git2`).

**Acceptance**: `cargo check -p git-forge-comment` passes with no errors.

---

## Step 2 — Define the domain types

**Context**: The design doc (`docs/design/git-forge-comments.md`) specifies that a comment is a Git commit object with trailers as metadata.
The data model that needs to be expressed in Rust:

- `AnchorType` — `Blob | Commit | Tree | CommitRange`
- `Anchor` — object SHA + type + optional line range + optional end SHA
- `Comment` — commit OID, anchor, author, timestamp, body, resolved flag,
  optional parent-comment OID (for replies), optional suggestion blob OID
- `NewComment` — parameters for creating a top-level comment
- `NewReply` — parameters for creating a reply (adds a second-parent OID)
- `Resolve` — parameters for resolving a comment thread

**Tasks**:

1. Replace `crates/git-forge-comment/src/lib.rs` with the domain types above.
   Match the style of `git-forge-issue`: plain structs + a trait (`Comments`) that `git2::Repository` will implement.
2. Define the `Comments` trait with these methods:
   - `fn comments_on(&self, ref_name: &str) -> Result<Vec<Comment>, git2::Error>`
   - `fn find_comment(&self, ref_name: &str, oid: git2::Oid) -> Result<Option<Comment>, git2::Error>`
   - `fn add_comment(&self, ref_name: &str, comment: &NewComment) -> Result<git2::Oid, git2::Error>`
   - `fn reply_to_comment(&self, ref_name: &str, reply: &NewReply) -> Result<git2::Oid, git2::Error>`
   - `fn resolve_comment(&self, ref_name: &str, resolve: &Resolve) -> Result<git2::Oid, git2::Error>`
3. Define the ref-name helper constants and builder functions:
   - `COMMENTS_REF_PREFIX: &str = "refs/forge/comments/"`
   - `fn issue_comments_ref(id: u64) -> String`
   - `fn review_comments_ref(id: u64) -> String`
4. Keep `pub mod git2;` declared but stub the file (empty `impl Comments for
   Repository {}`).

**Acceptance**: `cargo check -p git-forge-comment` passes.

---

## Step 3 — Implement `git2::Repository` backend

**Context**: `git-forge-core`'s comments module (`metadata/comments/git2.rs`) is all `todo!()`.
The `git-forge-comment` crate takes a different approach (commits as comment objects, not blobs-in-a-metadata-ref), so it gets its own fresh implementation.

**Tasks**:

Create `crates/git-forge-comment/src/git2.rs` implementing `Comments` for `git2::Repository`:

1. **`add_comment`** — use `git2::Repository::commit_tree` with the empty tree
   (`4b825dc6…`), set author/committer from `repo.signature()`, encode the body
   plus `Anchor:`, `Anchor-Type:`, optional `Anchor-Range:`, optional
   `Anchor-End:` as commit-message trailers, parent is the current tip of
   `ref_name` (or no parent if the ref does not exist yet), then
   `repo.reference()` / `repo.reference_ensure_log()` to update the ref tip.

2. **`reply_to_comment`** — same as above but with two parents: the current chain
   tip as first parent and the comment being replied to as second parent.

3. **`resolve_comment`** — same as `reply_to_comment` but append `Resolved: true` as a trailer.
   The `Resolve` struct carries the OID of the comment whose thread is being resolved.

4. **`find_comment`** — peel the ref to a commit, walk `--first-parent` until the
   target OID is found, parse trailers into a `Comment`.

5. **`comments_on`** — walk `--first-parent` from the ref tip, parse each commit
   into a `Comment`, return in reverse-chronological order.

6. **Trailer parsing helpers** — private functions `parse_trailers(msg: &str) ->
   Trailers` and `comment_from_commit(commit: &git2::Commit) -> Result<Comment,
   git2::Error>`.

**Acceptance**: `cargo test -p git-forge-comment` passes (even if there are no tests yet; no compile errors is sufficient at this stage).

---

## Step 4 — Tests

**Context**: `git-forge-issue` has integration tests in `src/tests/` that create a real in-memory `git2::Repository`.
Follow the same pattern.

**Tasks**:

Create `crates/git-forge-comment/src/tests.rs` (and declare `#[cfg(test)] mod tests;` in `lib.rs`) with the following test cases:

1. `add_comment_creates_ref` — call `add_comment` on a bare in-memory repo,
   assert the ref exists and the returned OID matches the ref tip.
2. `comments_on_returns_in_order` — add three comments, call `comments_on`,
   assert the slice has length 3 and timestamps are in the expected order.
3. `reply_sets_second_parent` — add a comment, reply to it, peel the reply
   commit and assert `parent_count() == 2` and `parent_id(1) == comment_oid`.
4. `resolve_sets_resolved_trailer` — add a comment, resolve it, call
   `comments_on`, find the resolution commit and assert `resolved == true`.
5. `find_comment_returns_none_for_missing` — call `find_comment` with a random
   OID, assert `Ok(None)`.

Use `tempfile::TempDir` + `git2::Repository::init` for repo setup, matching the pattern in `git-forge-issue/src/tests/`.

**Acceptance**: `cargo test -p git-forge-comment` passes.

---

## Step 5 — CLI definitions

**Context**: `git-forge-issue` exposes `pub mod cli` with a `clap::Subcommand` enum.
Follow the exact same pattern for `git-forge-comment`.

**Tasks**:

Create `crates/git-forge-comment/src/cli.rs` with:

```text
pub enum CommentCommand {
    /// Add a comment to an issue or review.
    Add {
        /// Target: "issue/<id>" or "review/<id>".
        target: String,
        /// Comment body (markdown). Reads from stdin if omitted.
        #[arg(short, long)]
        body: Option<String>,
        /// Blob SHA being commented on.
        #[arg(long)]
        anchor: Option<String>,
        /// Anchor type: blob, commit, tree, or commit-range.
        #[arg(long)]
        anchor_type: Option<String>,
        /// Line range, e.g. "42-47" (blob anchors only).
        #[arg(long)]
        range: Option<String>,
    },
    /// Reply to an existing comment.
    Reply {
        /// Target: "issue/<id>" or "review/<id>".
        target: String,
        /// OID of the comment to reply to.
        comment: String,
        /// Reply body (markdown). Reads from stdin if omitted.
        #[arg(short, long)]
        body: Option<String>,
    },
    /// Resolve a comment thread.
    Resolve {
        /// Target: "issue/<id>" or "review/<id>".
        target: String,
        /// OID of the comment to resolve.
        comment: String,
        /// Optional resolution message.
        #[arg(short, long)]
        message: Option<String>,
    },
    /// List comments.
    List {
        /// Target: "issue/<id>" or "review/<id>".
        target: String,
    },
}
```

**Acceptance**: `cargo check -p git-forge-comment` passes.

---

## Step 6 — Execution logic

**Context**: `git-forge-issue` has `pub mod exe` with an `Executor` wrapper and a `pub fn run(command: IssueCommand)` entry point.
Follow the same pattern.

**Tasks**:

Create `crates/git-forge-comment/src/exe.rs`:

1. `struct Executor(git2::Repository)` with `from_env()` and `repo()`.
2. Methods mapping to each `CommentCommand` variant:
   - `add_comment(target, body, anchor, anchor_type, range)`
   - `reply_to_comment(target, comment_oid_str, body)`
   - `resolve_comment(target, comment_oid_str, message)`
   - `list_comments(target)`
3. `pub fn run(command: CommentCommand)` — match and dispatch, print errors to
   stderr and `process::exit(1)` on failure. `add_comment` and `reply_to_comment`
   print the resulting OID to stdout. `list_comments` prints one comment per line
   as `<short-oid> [resolved] <first-line-of-body>`.
4. Target string helper: `fn parse_target(target: &str) -> Result<String,
   Box<dyn Error>>` that turns `"issue/7"` →
   `"refs/forge/comments/issue/7"` and errors on unrecognized prefixes.
5. Body helper: `fn read_body(body: Option<String>) -> Result<String, ...>` that
   reads stdin when `body` is `None`, matching the pattern in `exe.rs` of
   `git-forge-issue`.

**Acceptance**: `cargo check -p git-forge-comment` passes.

---

## Step 7 — Wire into top-level CLI

**Context**: `crates/git-forge/src/cli.rs` has `Commands` with `Issue`, `Review`, and `Release` variants.
`crates/git-forge/src/lib.rs` dispatches them.

**Tasks**:

1. In `crates/git-forge/src/cli.rs`, add:

   ```rust
   use git_forge_comment::cli::CommentCommand;
   ```

   and a new variant:

   ```rust
   /// Work with comments on issues and reviews.
   Comment {
       #[command(subcommand)]
       command: CommentCommand,
   }
   ```

2. In `crates/git-forge/src/lib.rs` (or `main.rs`, whichever dispatches), add an
   arm:

   ```rust
   Commands::Comment { command } => git_forge_comment::exe::run(command),
   ```

3. In `crates/git-forge/Cargo.toml`, add `git-forge-comment = { workspace = true }`.

**Acceptance**: `cargo build -p git-forge` succeeds.
Running `git forge comment --help` shows the comment subcommands.

---

## Step 8 — Integrate issue comments

**Context**: `Issues::add_issue_comment` in `git-forge-issue/src/git2.rs` is `todo!()`.
Now that `git-forge-comment` exists it can be implemented by delegating to the `Comments` trait.

**Tasks**:

1. Add `git-forge-comment = { workspace = true }` to
   `crates/git-forge-issue/Cargo.toml`.
2. Implement `Issues::add_issue_comment` in `git-forge-issue/src/git2.rs`:
   - Parse `author` as the comment author string.
   - Build a `NewComment` with `anchor` set to the issue ref itself (using
     `issue_ref(id)` as a logical anchor) and `body` as given.
   - Delegate to `self.add_comment(&issue_comments_ref(id), &new_comment)`.
3. Update `Issue::comments` field: populate it in `issue_from_ref` by calling
   `self.comments_on(&issue_comments_ref(id))`, converting each `Comment` to
   `(String, String)` (OID hex, body).

**Acceptance**: `cargo test -p git-forge-issue` passes.
The existing `add_issue_comment` stub is replaced and any test that exercises comments now works end-to-end.

---

## Step 9 — Commit

Commit all changes with:

```text
feat: implement `git-forge-comment` crate with full CLI and git2 backend

Adds the `git-forge-comment` crate: domain types, `Comments` trait,
`git2::Repository` implementation, CLI (`add`, `reply`, `resolve`,
`list`), and execution logic. Wires the `comment` subcommand into the
top-level `git forge` binary. Delegates `Issues::add_issue_comment`
to the new crate.

feat: add `git-forge-comment` crate with Comments trait and git2 impl
feat: add `git forge comment` CLI subcommand
feat: implement `Issues::add_issue_comment` via git-forge-comment
Assisted-by: Zed (Claude Sonnet 4.6)
```
