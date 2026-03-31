# git-forge comments v2

> The v1 design is archived at [`git-forge-comments-v1.md`](git-forge-comments-v1.md).

Comments are immutable commit objects in the Git object store.
Every comment belongs to exactly one **thread**.
Every thread lives on its own ref: `refs/forge/comments/<uuid-v7>`.

---

## Per-thread refs

```text
refs/forge/comments/<uuid-v7>   # one ref per thread (conversation)
refs/forge/index/comments-by-object  # server-rebuilt lookup index
```

A thread is a chronological chain of commits.
All commits for a thread — root, replies, resolutions, edits — live on the same ref.
There is no routing between chains on write.

---

## Data model

Each commit in a thread carries:

- **Tree**: non-empty (see Tree structure below)
- **Commit message**: comment body, followed by a blank line, then trailer block
- **First parent**: previous tip of the thread chain (chronological order)
- **Second parent** (optional): the commit being replied to, resolved, or edited (threading)
- **Author/committer**: identity and timestamp

---

## Anchor

Every comment anchors to a git object: a blob, a commit, or a tree.

```text
Anchor: <oid>
```

Optional fields for blob anchors with a line range:

```text
Anchor: <blob-oid>
Anchor-Range: <start>-<end>
```

The anchor OID is always present on every commit in the thread — root, replies, resolutions, edits.
Replies that don't specify a new anchor inherit and repeat the root's anchor.
This means the thread's tip commit alone is sufficient to find its anchor without walking the chain.

### Anchor semantics

| Anchor object type | Meaning |
|---|---|
| Blob | Inline code comment; line range is optional |
| Commit | Comment on an issue or review (anchor = that entity's root commit OID) |
| Tree | Comment on a directory |

Issue comments anchor to the issue's root commit OID.
Review comments anchor to the review's root commit OID.
Inline code comments anchor to a blob OID with an optional line range.

---

## Tree structure

Every comment tree has the following entries:

```text
100644 blob <sha>    body            # markdown comment text
100644 blob <sha>    anchor          # TOML: oid, optional start_line, end_line
100644 blob <sha>    context         # surrounding source lines (blob anchors with range only)
<mode> <type> <sha>  anchor-content  # the anchored git object, by OID
```

`anchor` blob contains TOML:

```toml
oid = "7e3f1a2b..."
start_line = 42   # omitted if no range
end_line = 47     # omitted if no range
```

`anchor-content` mode depends on the anchored object type:

| Object type | Mode |
|---|---|
| Blob | `100644` |
| Tree | `040000` |
| Commit | `160000` (gitlink) |

The `anchor-content` entry creates a reachability edge in the object graph, preventing GC of the anchored object.

---

## Trailer keys

| Trailer | Present on | Description |
|---|---|---|
| `Anchor` | every commit | OID of the anchored git object |
| `Anchor-Range` | blob anchors with line range | `"start-end"` |
| `Comment-Id` | every commit | UUID v7 of the thread (same across all commits in a thread) |
| `Resolved` | resolution commits | `"true"` |
| `Replaces` | edit commits | OID of the superseded comment |

---

## Second-parent semantics

The second parent has exactly one structural meaning: this commit is in response to that commit.
Two trailers modify display behavior:

- `Resolved: true` — treat this as a resolution of the target comment's thread
- `Replaces: <oid>` — treat this as an edit of the target comment; hide original, show edit

A commit with a second parent and neither trailer is a plain reply.

---

## Threading

Top-level comment (no second parent):

```text
C1  ←  first parent  ←  (chain tip)
```

Reply (second parent = comment being replied to):

```text
C2  ←  first parent  ←  C1
 ╲
  second parent  →  C1
```

`git log --first-parent refs/forge/comments/<uuid>` gives the flat chronological timeline.

---

## Discovery: comments-by-object index

`refs/forge/index/comments-by-object` is a signed commit pointing to a fanout tree:

```text
refs/forge/index/comments-by-object → signed commit → tree
├── 7e/
│   └── 3f1a2b...  →  blob: "019538a7-...\n019538b2-...\n"
├── af/
│   └── 3b2c4d...  →  blob: "019538c1-...\n"
```

Each leaf blob lists newline-separated thread UUIDs for comments anchored to that object OID.
The index maps all comments across all threads — roots and replies.

The index is rebuilt by the server's post-receive hook after each push that touches `refs/forge/comments/*`.
Clients consume it read-only.
If the index is absent or stale, `find_threads_by_object` falls back to scanning tip-commit `Anchor` trailers.

---

## Read-time reanchoring

When a blob-anchored comment's file has changed, reanchoring is done at read time using `git blame` plus fuzzy matching against the `context` blob.
There is no write-time migration.
No data is modified.

---

## Fetch strategy

All comment thread refs are fetched alongside the repository:

```text
[remote "origin"]
    fetch = +refs/forge/*:refs/forge/*
```

---

## Design principles

- **One ref per thread**: no routing, no migration, no topic scoping.
- **Anchors to any git object**: blob, commit, or tree.
- **GC safety via anchor-content**: every comment tree holds a reachability edge to its anchored object.
- **Read-time reanchoring**: context blob + blame, no write-time mutation.
- **Server-rebuilt index**: clients query; server rebuilds after push.
- **Portable**: everything is in the Git object store.
  Clone the repo, you have the comments.
