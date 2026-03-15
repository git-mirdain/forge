# git-forge comments

Comments are immutable commit objects in the Git object store.
They anchor to any Git object — blobs, commits, trees, or commit ranges — and support threading through the DAG.

## Data model

A comment is a commit with:

- **Tree**: the empty tree by default (`4b825dc642cb6eb9a060e54bf899d15b4fdd19d0`), or a non-empty tree carrying structured payloads (suggested changes, attachments, preserved anchor content)
- **Commit message**: the comment body, with anchor metadata as trailers
- **First parent**: the previous tip of the comment chain (chronological ordering)
- **Second parent** (optional): the comment being replied to (threading)
- **Author/committer fields**: identity and timestamp

Most comments are one object (the commit).
Comments with payloads add a tree and blobs.

## Ref structure

```text
refs/forge/comments/issue/<id>       # comments on issues
refs/forge/comments/review/<id>      # comments on code reviews
```

Each ref points to the tip of a chronological chain.
All comments for a given topic — top-level and replies — live on the same ref.

## Anchoring

Comments anchor to arbitrary Git objects via trailers in the commit message.

### Anchor to a commit

```text
This approach introduces a race condition

Anchor: af3b2c4d
Anchor-Type: commit
```

### Anchor to a blob with line range

```text
Off-by-one error here

Anchor: 7e3f1a2b
Anchor-Type: blob
Anchor-Range: 42-47
```

The blob SHA pins the comment to an exact file version.
Line numbers are stable because the blob is immutable.
To determine whether the comment is still current, resolve the file path in HEAD and compare blob SHAs:

```bash
git rev-parse HEAD:src/main.rs
```

If it matches `Anchor`, the comment is current.
If not, it's outdated.

### Anchor to a tree

```text
This directory structure is confusing

Anchor: 9c4d2e1f
Anchor-Type: tree
```

### Anchor to a commit range

```text
This series of changes breaks the API contract

Anchor: a1b2c3d4
Anchor-Type: commit-range
Anchor-End: e5f6a7b8
```

### Summary of trailers

| Trailer | Required | Description |
|---|---|---|
| `Anchor` | yes | SHA of the target object |
| `Anchor-Type` | yes | `blob`, `commit`, `tree`, or `commit-range` |
| `Anchor-Range` | no | Line range (e.g. `42-47`), for blobs only |
| `Anchor-End` | no | End SHA, for commit ranges only |
| `Resolved` | no | `true` when a comment resolves its thread |

## Threading

Threading is structural, encoded in the DAG via second parents.

A top-level comment has one parent (the chain tip):

```text
C2  ←  first parent  ←  C1
```

A reply has two parents — the chain tip and the comment it replies to:

```text
C3  ←  first parent  ←  C2  ←  C1
 ╲
  second parent  →  C1
```

Walking first parents gives the flat chronological timeline.
Walking second parents from any comment gives its thread ancestry.

### Example DAG

```text
refs/forge/comments/review/60  →  C5
                                   │ ╲
                              1st  │  ╲ 2nd (reply to C2)
                                   │   ╲
                                  C4    C2
                                   │
                                  C3 (reply to C1, has 2nd parent → C1)
                                   │
                                  C2
                                   │
                                  C1
```

`git log --first-parent refs/forge/comments/review/60` shows: C5, C4, C3, C2, C1.

Thread view of C5: C5 → C2 (via second parent).
Thread view of C3: C3 → C1 (via second parent).

## Tree payloads

Most comments use the empty tree.
When a comment carries structured data, it uses a non-empty tree.
The convention is to check whether the tree SHA equals the empty tree — if not, inspect it for known entries.

### Suggested changes

A review comment can propose a code fix by including the replacement content as a blob:

```text
100644 blob <sha>    suggestion
```

The blob contains the replacement text for the anchored line range.
Tooling applies it with:

```bash
SUGGESTION=$(git cat-file -p <comment-tree-sha> | awk '/suggestion/{print $3}')
git cat-file -p $SUGGESTION  # the replacement code
```

`git forge apply <comment-sha>` extracts the suggestion blob, reads the `Anchor` and `Anchor-Range` trailers, and patches the file.
This is GitHub's "suggested changes" feature, but in the object store.

### Attachments

Images, logs, or supplementary files:

```text
100644 blob <sha>    attachments/screenshot.png
100644 blob <sha>    attachments/error.log
```

### Preserved anchor content

If a comment anchors to a blob via trailer and that blob's commit gets rebased away, the blob may become unreachable and get garbage collected.
Including the anchored blob in the comment's tree prevents this:

```text
100644 blob <sha>    anchor-content
```

The comment now literally preserves the content it's commenting on.
Self-contained and GC-safe.
The `anchor-content` entry's SHA should match the `Anchor` trailer.

### Creating a comment with a suggestion

```bash
EMPTY_TREE=4b825dc642cb6eb9a060e54bf899d15b4fdd19d0
TIP=$(git rev-parse refs/forge/comments/review/60)

# Create the suggestion blob
SUGGESTION_BLOB=$(echo 'for i in 0..items.len() {' | git hash-object -w --stdin)

# Build the tree
TREE=$(printf '100644 blob %s\tsuggestion\n' "$SUGGESTION_BLOB" | git mktree)

# Create the comment with the non-empty tree
COMMENT=$(git commit-tree $TREE \
  -p $TIP \
  -m "Off-by-one: should be 0..len() not 1..len()

Anchor: 7e3f1a2b
Anchor-Type: blob
Anchor-Range: 42-42")

git update-ref refs/forge/comments/review/60 $COMMENT
```

## Plumbing

### Create a top-level comment

```bash
EMPTY_TREE=4b825dc642cb6eb9a060e54bf899d15b4fdd19d0
TIP=$(git rev-parse refs/forge/comments/review/60)

COMMENT=$(git commit-tree $EMPTY_TREE \
  -p $TIP \
  -m "Off-by-one error here

Anchor: 7e3f1a2b
Anchor-Type: blob
Anchor-Range: 42-47")

git update-ref refs/forge/comments/review/60 $COMMENT
```

### Create a reply

```bash
REPLY=$(git commit-tree $EMPTY_TREE \
  -p $(git rev-parse refs/forge/comments/review/60) \
  -p $COMMENT \
  -m "Agreed, also line 45 has the same bug

Anchor: 7e3f1a2b
Anchor-Type: blob
Anchor-Range: 45-45")

git update-ref refs/forge/comments/review/60 $REPLY
```

### Resolve a thread

Resolving a comment is itself a comment:

```bash
RESOLVE=$(git commit-tree $EMPTY_TREE \
  -p $(git rev-parse refs/forge/comments/review/60) \
  -p $COMMENT \
  -m "Fixed in a1b2c3d4

Resolved: true
Anchor: 7e3f1a2b
Anchor-Type: blob
Anchor-Range: 42-47")

git update-ref refs/forge/comments/review/60 $RESOLVE
```

### Create the first comment on a new topic

When no ref exists yet, the first comment has no parent:

```bash
FIRST=$(git commit-tree $EMPTY_TREE \
  -m "This needs a rewrite

Anchor: af3b2c4d
Anchor-Type: commit")

git update-ref refs/forge/comments/issue/7 $FIRST
```

### Read all comments

```bash
git log --format='%H %P%n%B' refs/forge/comments/review/60
```

### Extract anchors

```bash
git log --format='%H %(trailers:key=Anchor,valueonly)' \
  refs/forge/comments/review/60
```

## Indexing

### Reverse index

On startup, build a map from anchor SHA to comment SHAs by streaming all comment refs:

```bash
git log --format='%H %(trailers:key=Anchor,valueonly)%(trailers:key=Anchor-Range,valueonly)' \
  refs/forge/comments/issue/* refs/forge/comments/review/*
```

For 10,000 comments this runs in milliseconds.
Store in memory as a hash map.

### Invalidation

Watch ref tips.
When a ref changes after fetch, walk from the new tip until hitting a known SHA.
Index only the new commits.

### `git forge blame` integration

1. Run `git blame -C <file>` — get originating commit per line
2. Resolve `<commit>:<path>` to blob SHA
3. Look up blob SHA in reverse index
4. Filter by `Anchor-Range` containing the line
5. Filter out resolved comments
6. Display inline

`git blame -C` tracks code movement across renames and copies.
Comments follow automatically because the originating commit and blob SHA are unchanged.

## Offline divergence

Two people comment offline on the same topic.
Each creates commits parented to the tip they last saw.
On push, one succeeds and the other gets a conflict.

Resolution: fetch, then create a merge commit reconciling the two chains.
This is standard Git — the DAG handles it.

```bash
git fetch origin refs/forge/comments/review/60:refs/forge/comments/review/60
# If diverged, local tip and remote tip are different
git merge-base --is-ancestor $LOCAL_TIP $REMOTE_TIP || {
  MERGE=$(git commit-tree $EMPTY_TREE \
    -p $REMOTE_TIP \
    -p $LOCAL_TIP \
    -m "Merge comment histories")
  git update-ref refs/forge/comments/review/60 $MERGE
}
```

## Transfer

Comments live outside `refs/heads/*` and `refs/tags/*`, so they are not fetched by default.
Configure remotes:

```text
[remote "origin"]
    fetch = +refs/forge/*:refs/forge/*
```

Or fetch explicitly:

```bash
git fetch origin refs/forge/comments/*:refs/forge/comments/*
git push origin refs/forge/comments/*:refs/forge/comments/*
```

## Scaling

The bottleneck is ref count, not object count.
Each topic is one ref.
For most projects, this is hundreds to low thousands — no problem.

If ref count becomes an issue:

- Archive closed topics by compacting their chains into `refs/forge/archive/<year>`
- Fan out refs: `refs/forge/comments/review/ab/<id>` (same strategy Git uses for objects)
- Persist the reverse index to disk; rebuild incrementally

A forge-aware server can store comments in a database and serve them as synthetic refs via a custom `git-upload-pack`, making ref enumeration a non-issue while preserving full Git protocol compatibility.
Clients don't change.

## Design principles

- **One object per comment by default**: a commit with the empty tree.
  Comments with payloads (suggestions, attachments) use a non-empty tree and add blobs as needed.
- **Threading is structural**: second parents, not metadata.
- **Anchoring is metadata**: trailers, not parents (parents can only point to commits).
- **Immutable**: comments are never edited.
  Corrections are new comments.
  Resolution is a new comment with a `Resolved: true` trailer.
- **Portable**: everything is in the Git object store.
  Clone the repo, you have the comments.
  No external database required.
