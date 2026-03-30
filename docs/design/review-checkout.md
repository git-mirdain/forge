# Review Checkout via Worktree + Neovim Plugin

## Context

Today, every `forge comment add` requires `--review <ref>`.
The goal: `forge review checkout <ref>` creates a git worktree with the review's head checked out, and that worktree *is* the active review context — no flags needed on subsequent comment commands.
A minimal Neovim plugin provides editor integration for adding comments.

## Part 1: `forge review checkout`

### CLI

```text
forge review checkout <reference> [path]
```

- `reference`: display ID or OID prefix
- `path`: worktree location (default: `../<repo-name>.review/<reference>`)

### Mechanics

1. Resolve review via `store.get_review(reference)`
2. Create a git worktree at the target path, detached at `target.head`
3. Write `forge-review` marker (containing the review OID) into the
   worktree's gitdir (`<repo.path()>/worktrees/<name>/forge-review`)
4. Print the path so the user can `cd` into it

### Active review detection

```rust
fn active_review(&self) -> Option<String> {
    if !self.repo.is_worktree() { return None; }
    let marker = self.repo.path().join("forge-review");
    std::fs::read_to_string(marker).ok().map(|s| s.trim().to_string())
}
```

### Comment context fallback

The `--issue`/`--review` entity group on all `CommentCommand` variants becomes optional.
When neither is provided, the executor calls `active_review()` and uses the result as the implicit `--review` value.
Errors if no context is found and no flag was passed.

## Part 2: Neovim Plugin

### Location

`extensions/forge-nvim/lua/forge.lua` — single file, installable via lazy.nvim pointing at `extensions/forge-nvim`.

### Workflow

1. User is in a review worktree, editing code
2. Visual-select lines (or cursor on a line) and press `<leader>fc`
3. Centered float opens (markdown filetype) with title showing `file:range`
4. Write comment, `ZZ` to submit, `q`/`<Esc>` to cancel
5. Plugin writes buffer to temp file, runs:
   `forge comment add --anchor <blob-oid> --anchor-path <relpath> --range <start>-<end> -f <tmpfile>`
6. No `--review` needed — forge detects from worktree context

### Keymaps

| Key | Mode | Action |
|-----|------|--------|
| `<leader>fc` | n/v | Add comment (visual = range, normal = single line) |
| `<leader>fr` | n | Reply to comment (prompts for OID) |
| `<leader>fl` | n | List comments in loclist |
| `<leader>fR` | n | Resolve thread (prompts for OID) |

### Anchor detection

- Blob OID: `git rev-parse HEAD:<relative-path>`
- Relative path: strip git root from buffer path
- Range: visual selection or current line
