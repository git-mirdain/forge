+++
title = "Forge: A Git-Native Development Platform"
subtitle = "Design Specification"
version = "0.2.0"
date = 2026-03-11
status = "Draft"
+++

# Forge: A Git-Native Development Platform

## Foundation

Forge is a development platform built entirely on Git primitives. Issues, code review, comments, approvals, access control, releases, and enforcement are stored as Git objects — refs, trees, blobs, and commits. There is no database. The Git repository is the database.

The design has two principles. First, every piece of state is a Git object, reachable from a ref, signed where authorship matters. Second, the data model separates entities from annotations. Entities (issues, reviews) are standalone refs with their own lifecycles. Annotations (comments, approvals) are metadata on objects — attached to the content they describe, not to the event that prompted them. Relationships between entities are standalone metadata, not embedded fields.

Forge depends on [`git-metadata`](https://github.com/git-ents/git-metadata) for annotations and relational metadata. `git-metadata` extends Git's notes feature to tree-structured data, allowing arbitrary metadata to be attached to any object (blob, tree, or commit) without modifying history.


## Entity IDs and Counters

Entities (issues, reviews) use sequential integer IDs. A counter ref tracks the next available ID per entity type:

```
refs/meta/counters → commit → tree
├── issues          # plain text: "47"
├── reviews         # plain text: "103"
```

Incrementing is a signed commit to this ref. The counter's history is its own audit log.

### Assignment Protocols

Three protocols produce identical refs. The choice depends on the server environment.

**Pure Git (no server logic).** Optimistic concurrency via `git push --atomic` and `--force-with-lease`:

1. Fetch `refs/meta/counters`, read current value N.
2. Create the entity commit locally.
3. Commit N+1 to the counter ref locally.
4. `git push --atomic --force-with-lease=refs/meta/counters:<expected-oid> refs/meta/counters refs/meta/issues/<N+1>`
5. If rejected (someone else incremented), retry from step 1.

`--atomic` ensures all ref updates succeed or none do. `--force-with-lease` on the counter ref makes it a compare-and-swap. Together: atomic multi-ref CAS with no server-side logic.

**Server hooks (smart server).** The client pushes to an inbox ref. The server's post-receive hook reads the commit, assigns the next integer, creates the canonical ref, and deletes the inbox ref:

1. User pushes to `refs/inbox/<fingerprint>/issues/<anything>`.
2. Server post-receive hook reads it, assigns the next integer, creates `refs/meta/issues/<N+1>`, deletes the inbox ref.
3. Returns the assigned ID to the user (post-receive hook output goes back to the pusher over the transport).

**UI server (direct repo access).** The UI has filesystem access. Lock the counter file, increment, write the entity ref, unlock. No CAS retry loop, no hooks.

External contributors always use the inbox path — they cannot write to `refs/meta/counters`.


## Annotations

### Comments

A comment is an annotation on a blob, anchored to a line range. Comments are repo-wide — they are not owned by a review or an issue. A review may prompt someone to leave a comment, but the comment exists independently and persists as long as the code it describes exists.

Comments are stored as `git-metadata` entries on blob oids:

```
refs/metadata/comments   (fanout by blob oid)
  <blob-oid>/
    <comment-id>/
      meta          # toml: author (fingerprint), timestamp, start_line,
                    #       end_line, context_lines
      body          # markdown
      resolved      # toml: by, timestamp — presence means resolved
      reply/
        001         # toml: author, timestamp, body
        002
```

The `context_lines` field stores a few lines surrounding the anchored range. This is the fallback for relocation when blame is ambiguous.

Relationships to other entities (issues, reviews, commits) are stored as relational metadata, not embedded in the comment. See the Relational Metadata section.

#### Reanchoring

Comments are anchored to a specific blob oid and line range. When a new commit changes a file, the anchored blob oid no longer appears in the current tree. The comment must be reanchored to the new blob.

A reanchoring process (server-side post-receive hook, local daemon, or explicit CLI command) runs on every new commit:

1. Diff the commit to get changed files.
2. Filter to files with existing comments (index lookup by blob oid).
3. Blame those files in the new tree.
4. For each comment on an old blob oid, find where those lines now live in the new blob.
5. Write a new anchor: update the comment's metadata with the new blob oid and line range.

Each reanchoring is a new commit on the `refs/metadata/comments` ref. The commit history records where the comment has traveled.

If blame says the lines were deleted, the comment becomes orphaned. This is meaningful — the code was removed or rewritten. An orphaned comment surfaces as "detached" in the UI. Someone should explicitly resolve it or the author of the change should address it.

Same-file blame only. Cross-file moves (extractions, renames) orphan the comment. This is a conservative default. Chasing moves across files relies on blame heuristics (`-C`) that can silently misattribute. Better to orphan and let a human re-anchor than to silently drift to wrong code.

The cost of reanchoring is proportional to `(files changed in commit) ∩ (files with comments)`. This is almost always a small set.

If the daemon is not running, comments still work. They remain anchored to their last-known blob oid and show as stale until something reanchors them. No correctness problem, just degraded experience.

#### Querying

The hot path is "show me all comments on this file." The viewer:

1. Blames the current file — gets (blob oid, line range) pairs for every line.
2. Collects the set of blob oids.
3. Reads `refs/metadata/comments` for entries matching those oids.
4. Maps each comment's line range onto current file positions.

This requires an index from blob oid to comment entries. The `git-metadata` fanout structure provides this directly — the tree is keyed by blob oid.


### Approvals

Forge supports four levels of approval, each attesting to a different thing:

| Level | Object | Meaning |
|-------|--------|---------|
| File | blob OID | "This file is correct" |
| Tree | tree OID | "This subtree is correct" |
| Commit | patch-id | "This change is correct" |
| Range | range patch-id | "This overall change is correct" |

#### Change Approvals (patch-id and range patch-id)

A change approval is an annotation on a patch-id. Using patch-id rather than commit oid means approvals survive rebases automatically — the same change before and after rebase produces the same patch-id.

```
refs/metadata/approvals   (fanout by patch-id)
  <patch-id>/
    <fingerprint>       # toml: timestamp, type ("patch"|"range"), message (optional)
```

The metadata commit adding the entry is signed by the approver. The approval is verifiable from the commit signature.

`git patch-id` is a Git built-in. It hashes the diff content, ignoring line numbers and whitespace. Two commits that represent the same change produce the same patch-id regardless of where they appear in history.

Range patch-id is computed over the full diff of a commit range: `git diff base..tip | git patch-id`. This produces a single hash for the overall change, regardless of how many commits compose it. Squashing the range produces the same range patch-id because the overall diff is identical.

Behavioral properties:

- **Rebase with no conflict:** same patches, same patch-ids, approvals carry over automatically. Same range diff, same range patch-id, range approval carries over.
- **Rebase with conflict resolution:** affected patches produce new patch-ids. Overall diff changes, new range patch-id. Both need re-approval.
- **Squash:** new combined diff per commit, new per-commit patch-id. But range patch-id unchanged — overall diff is the same.
- **Amend commit message only:** patch-id unchanged, approval survives.
- **Reorder commits without changing overall diff:** individual patch-ids may change, range patch-id survives.

When a reviewer approves a review (the coordination entity), the tooling bulk-approves every patch-id in the review's commit range and writes a range patch-id approval. One storage model. The review is the UX; the approvals are the data. Policy decides which level to enforce.

#### State Approvals (blob OID and tree OID)

A state approval attests to the correctness of a file or subtree at a specific point in time. It is an annotation on the blob or tree OID.

```
refs/metadata/approvals   (fanout by oid)
  <blob-or-tree-oid>/
    <fingerprint>       # toml: timestamp, type ("blob"|"tree"), path, message (optional)
```

The `path` field records which file or directory was approved (a blob OID alone doesn't indicate location). State approvals break on any change to the approved object — the OID changes, so a new approval is required.

Tree approvals are the monorepo primitive. A subtree OID changes when anything under it changes, so it automatically invalidates. Team A approves `services/auth/` as a tree OID; any change under that path produces a new tree OID requiring re-approval.

#### Approval Policy

The merge gate checks whichever approval level policy requires:

```toml
[branches.main]
merge_strategy = "squash"
approval_check = "range"              # check range patch-id
min_approvals = 1
exclude_author = true                 # author cannot self-approve
block_unresolved_comments = true

[branches.main.state_approval]
paths = ["crypto/*", "services/auth/*"]
type = "tree"                         # require tree approval for these paths
approvers = ["@crypto-team", "@auth-team"]

[branches.develop]
merge_strategy = "rebase"
approval_check = "per_patch"          # check individual patch-ids
min_approvals = 1
```

In permissive mode (no approval enforcement), approvals are recorded but not enforced. The data is always granular. A team switching from permissive to strict doesn't change their workflow — the approvals were already being recorded.

When a reviewer approves a review, the tooling bulk-writes approval entries for every patch-id in the review's commit range plus a range approval. Individual patch-level or state-level approval is also possible for surgical sign-off.

Teams can approve asynchronously. Auth team approves their subtree, billing team approves theirs, merge proceeds when all required approvals are satisfied.


## Relational Metadata

Relationships between entities are stored as `git-metadata` trees, not embedded in entity fields. Both directions are first-class — no scanning, no derived indexes.

```
refs/metadata/links/<entity-type>/<entity-id>/
  <related-type>:<related-id>    # blob: optional metadata about the relation
```

Both directions are stored:

```
refs/metadata/links/issues/42/
  comment:abc123          # "comment abc123 references issue 42"
  review:7                # "review 7 references issue 42"

refs/metadata/links/comments/abc123/
  issue:42                # reverse direction
```

Each link is a Git tree entry. Adding a link writes both directions in a single commit on the metadata ref — one atomic operation.

Relationships are independently authored and signed. "User X linked comment C to issue 42" is a distinct, attributable action — not a side effect of editing a comment.

Reverse lookups are tree reads, not index scans. "All comments referencing issue 42" is a tree listing of `refs/metadata/links/issues/42/`.


## Entities

### Issues

An issue is a standalone ref with its own lifecycle. It is not metadata on any object — it is an entity.

```
refs/meta/issues/<issue-id> → commit → tree
├── meta            # toml: author, title, state, labels, assignees, created
├── body            # markdown
├── comments/
│   ├── 001-<ts>-<fingerprint>      # markdown
│   ├── 002-<ts>-<fingerprint>
│   └── ...
```

Each mutation — state change, new comment, label update, assignment — is a new commit on the issue's ref. The commit history is the issue's audit log. `git log refs/meta/issues/<id>` shows every change, who made it, and when.

Issue comments are conversation within the issue. They are not the same as code comments. Code comments are annotations on blobs, visible everywhere that content appears, blame-reanchored. Issue comments are part of the issue's history, scoped to the issue.

Relationships to other entities (commits, blob+line ranges, other issues, reviews) are stored as relational metadata, not embedded in the issue.

Ref-per-issue eliminates write contention. Two people editing different issues never conflict.

#### Labels, Assignment, State

Labels are a list of strings in `meta`. No separate taxonomy system. A label exists when someone uses it.

Assignment is a list of fingerprints in `meta`.

State is an enum in `meta`: `open`, `closed`. No intermediate states in the core model. Extensions or conventions can add them via labels.


### Reviews

A review is a coordination entity — "please look at commits X..Y." It references commits but is not metadata on any commit. It has its own lifecycle independent of the commits it covers.

```
refs/meta/reviews/<review-id> → commit → tree
├── meta            # toml: author, target_branch, state, created
├── description     # markdown
├── revisions/
│   ├── 001         # toml: head_commit, timestamp
│   ├── 002         # toml: head_commit, timestamp
│   └── ...
```

The `revisions/` entries record each time the author pushed new commits. This provides "review rounds" — a reviewer can see what changed between revision 1 and revision 2.

State is `open`, `merged`, or `closed`.

A review does not contain comments or approvals. It prompts them. Comments land on blob oids (via `git-metadata`). Approvals land on patch-ids, range patch-ids, blob oids, or tree oids (via `git-metadata`). The review is how you discover which commits to look at. The comments and approvals are what you find when you look.

This means comments and approvals outlive the review that prompted them. A comment on line 42 of `lib.rs` persists and follows that code regardless of which review prompted it. Closing or merging a review does not resolve its comments.


## Checks

### Check Definitions

Check definitions live in the repository, versioned with the code:

```
.forge/checks/
├── build.toml
├── lint.toml
└── test.toml
```

```toml
# build.toml
name = "build"
image = "rust:1.85"
run = "cargo build --release"
triggers = ["refs/heads/*", "refs/meta/queue/*"]
secrets = [
  { name = "CARGO_REGISTRY_TOKEN", type = "file" },
]
```

The check definition that runs is the one at the commit being checked — no external configuration that drifts from the code.

### Check Results

Check results are metadata on commits, keyed by run ID to support multiple runs and matrix builds:

```
refs/metadata/checks/<commit-oid>/
  <run-id>/
    meta          # toml: name, state (pass|fail|running), started, finished,
                  #       runner_fingerprint, params (optional)
    log           # blob: raw output
```

`run-id` is a timestamp + fingerprint or a short random ID. Every execution is a distinct tree entry. Never overwrites.

Matrix builds use the `params` field to distinguish variants:

```toml
name = "build"
state = "pass"
params = { os = "linux", arch = "amd64" }
```

Reruns are new entries. The old run's result is permanent history. The merge gate queries all runs for a required check name and uses the most recent result.

### Runners

A runner is a contributor with a runner role. It signs its results. The check result commit on the metadata ref is signed by the runner's key.

```toml
[roles.runner]
push = ["refs/metadata/checks/*"]
approve = false
```

Runners poll a queue (the queue primitive already exists) or get notified by the server's post-receive hook.

### Merge Gate Integration

Policy declares required checks per branch:

```toml
[branches.main]
require_checks = ["build", "test"]
```

The pre-receive hook reads `refs/metadata/checks/<oid>` for the commits being pushed and verifies the required checks passed.

Check results do not survive rebase. A rebased commit is a new OID, so it needs to be rechecked. This is correct — the rebase could introduce failures.

### Local Execution

`git forge check run build` executes locally using the same definition. Same inputs, same container image, reproducible. The only difference is who signs the result. Policy can require runner-signed results for merge but allow local runs for feedback.

### Check Policy Querying

The checks required to push to any branch are queryable from the repo itself:

```sh
git show main:.forge/policy.toml    # required checks for main
git show main:.forge/checks/build.toml  # what "build" does
```

No server query, no API. Policy and check definitions are versioned with the code.


## Secrets

Secrets cannot be Git objects. Git repos get cloned, forked, mirrored. A secret in a ref is a secret on every machine that fetches.

### Design

Check definitions reference secrets by name. Names are in Git; values are not.

```toml
# .forge/checks/deploy.toml
secrets = [
  { name = "AWS_ACCESS_KEY", type = "file" },
  { name = "DEPLOY_TOKEN", type = "file" },
]
```

The secret store is per-server, not per-repo. Entries are encrypted at rest with a key derived from the server's own identity:

```
/var/forge/secrets/<repo>/
  AWS_ACCESS_KEY          # encrypted blob
  DEPLOY_TOKEN            # encrypted blob
  meta                    # toml: who set it, when, ACL
```

The ACL specifies which runner fingerprints can read each secret.

### Injection

The server injects secrets, not the runner. This prevents a compromised runner from requesting arbitrary secrets.

1. Runner picks up a job from the queue and authenticates to the server with its key.
2. Server reads the check definition at that commit's tree itself, verifies the runner's fingerprint is in the ACL for each listed secret.
3. Server writes secrets to a tmpfs volume mounted into the container at `/run/forge/secrets/<name>`.
4. The mount is read-only. The container has no capability to remount. Network egress is restricted to what the check definition declares.
5. Runner spawns the container, executes, signs the result, pushes result metadata. Secrets are destroyed when the container exits.

tmpfs is memory-backed, never hits disk. No environment variable exposure, no `/proc/<pid>/environ` leakage, no child process inheritance, no accidental logging.

The trust boundary is the check definition in the repo. A PR that adds `secrets = [{ name = "PROD_DB_PASSWORD", type = "file" }]` to a check is visible and reviewable. If someone merges a check that exfiltrates secrets, no runtime mechanism saves you. The tmpfs mount eliminates accidental leaks, which are the common case.

### Management

```sh
git forge secret set AWS_ACCESS_KEY --value=...
git forge secret set DEPLOY_TOKEN --file=token.txt
git forge secret list
git forge secret grant AWS_ACCESS_KEY --runner=<fingerprint>
```

These are API calls to the server, not ref writes. This is an honest exception to the "everything is a ref" principle.

### Audit Trail

Every secret read/write/grant event is logged to a server-maintained ref:

```
refs/meta/audit/secrets → commit → tree
├── 001-<ts>    # toml: action, secret_name, actor_fingerprint
├── 002-<ts>
```

Secret values are opaque. The history of who touched what is in Git and signed.


## Access Control

### Contributors

Contributors are stored in a ref:

```
refs/meta/contributors → commit → tree
├── <fingerprint>/
│   ├── key.pub         # SSH or GPG public key
│   ├── meta            # toml: name, email, added_by, timestamp
│   └── roles           # toml: list of role names
```

Adding a contributor is a signed commit to this ref. The commit must be signed by someone with permission to modify contributors (governed by policy).

The first commit is self-signed by the project creator. This bootstraps trust.

### Identity (Future)

Long-term, contributors reference an external identity repository rather than embedding keys directly. The identity repo is controlled by the user and publishes their current keys. The project trusts the identity repo, not individual keys. Key rotation happens in one place and propagates to all projects.

Near-term, the key in the contributors ref is the identity. The fingerprint is the stable identifier. No registration, no accounts. A key pair is sufficient.

### Roles and Policy

Roles are names. Policy maps roles to permissions:

```toml
# policy.toml

[roles.admin]
push = ["refs/heads/*"]
approve = true
manage_issues = true
modify_contributors = true
modify_policy = true

[roles.maintainer]
push = ["refs/heads/*"]
approve = true
manage_issues = true

[roles.contributor]
push = ["refs/heads/feat/*"]
approve = false
manage_issues = true

[roles.reviewer]
push = []
approve = true
manage_issues = false

[roles.runner]
push = ["refs/metadata/checks/*"]
approve = false
```

Path-scoped permissions restrict which files a role can modify:

```toml
[roles.frontend]
push = ["refs/heads/*"]
paths = ["web/*", "css/*"]
```

Enforced in the pre-receive hook via `git diff --name-only <old> <new>`.

### Self-Protecting Policy

`policy.toml` is committed to the repository. Changing it requires meeting the rules currently in effect. You cannot weaken policy without satisfying the current policy's review and approval requirements.

```toml
[access.contributors]
modify = { require_role = "admin" }

[access.policy]
modify = { require_role = "admin", require_review = true }
```

### External Contributors

External contributors have no key in `refs/meta/contributors`. They cannot push to the repo directly.

The pre-receive hook allows anyone with a valid signature to push to a scoped inbox namespace:

```
refs/inbox/<fingerprint>/<branch-name>
```

The hook verifies the push is signed and that the target ref is under the pusher's own fingerprint prefix. No role needed. They can only write to their own namespace.

A maintainer reviews the code at that ref and decides whether to merge. The inbox namespace is garbage-collected after resolution.

### Ref Visibility

Git's transport protocol advertises all refs on fetch. Hiding refs from unauthorized users requires a server-side filter.

Git natively supports HTTPS via the smart HTTP protocol. The ref advertisement is the response to `GET /info/refs?service=git-upload-pack`. An HTTP middleware in front of `git-http-backend`:

1. Authenticates the user.
2. Looks up their role.
3. Filters the ref advertisement based on permissions.
4. Proxies everything else unchanged.

Public refs (issues, comments) are visible to everyone. Private refs (policy details, contributor roles) are visible only to members with the appropriate role.

For pure Git over SSH without a custom server, all refs are visible to anyone with read access. This is an honest limitation of the protocol.


## Enforcement

### Merge Gate

The pre-receive hook enforces policy on pushes to protected branches:

1. Verify push is signed by a known contributor.
2. Check the contributor's role permits pushing to this ref.
3. `git diff --name-only <old> <new>` — check path-scoped permissions if configured.
4. `git log <old>..<new>` — list commits in the range.
5. Check approvals per policy:
   - If `approval_check = "per_patch"`: `git patch-id` each commit, check `refs/metadata/approvals/<patch-id>`.
   - If `approval_check = "range"`: compute `git diff <old>..<new> | git patch-id`, check range approval.
   - If `state_approval` configured: extract blob/tree OIDs for specified paths from the merge commit, check for matching approvals.
6. Check `refs/metadata/checks/<oid>` for required checks on each commit.
7. Optionally check for unresolved comments on affected blob oids.
8. Accept or reject with a specific failure message listing what's missing.

### Merge Queue

The merge queue is a ref containing an ordered list of entries:

```
refs/meta/queue/merge → commit → tree
├── 001-<review-id>     # toml: head_commit, submitted_by, timestamp
├── 002-<review-id>
└── ...
```

The server processes entries in order:

1. Take the first entry.
2. Rebase onto current target branch HEAD.
3. Trigger a build on the rebased commit.
4. On success, push to the target branch.
5. On failure, reject and notify. Advance to next entry.

Rebase before build is essential — a branch that passed checks against an older HEAD may fail against current HEAD.

Batching: the processor can rebase multiple entries as a stack, test the combined result, and push all on success. On failure, bisect the batch to find the breaker.

Write serialization uses the pre-receive hook, which runs atomically one push at a time.

### Queue as a Primitive

The merge queue is an instance of a general queue:

```sh
git forge queue create <name>
git forge queue push <name> <ref>
git forge queue pop <name>
git forge queue list <name>
```

Processing hooks declare what happens when entries appear. The merge queue's hook rebases and tests. A CI queue's hook executes build actions. A release pipeline chains queues.


## Metadata Push and Auto-Merge

### Server Auto-Merge

Metadata refs are almost perfectly auto-mergeable. Most metadata operations add new entries at unique paths in a tree. Two people adding different comments, approvals, or links touch disjoint tree paths.

On non-fast-forward push to a metadata ref, the server:

1. Finds the merge base between the incoming commit and the current ref tip.
2. Performs a three-way tree merge.
3. If clean, creates a merge commit, updates the ref. Returns success to the pusher.
4. If conflicting (same path modified both sides), rejects with a message listing conflicting paths.

This is `git merge-tree` (or the equivalent plumbing), a Git built-in.

What conflicts in practice: two people resolving the same comment simultaneously (both write to `<comment-id>/resolved`), or two people editing the same entity's `meta` file. Rare, and the right answer is rejection — the slower writer retries and sees the current state.

What never conflicts: adding comments (unique IDs → unique paths), adding approvals (unique fingerprints → unique paths), adding links (unique paths per direction), adding replies (sequenced by timestamp in the filename). That's the vast majority of metadata operations.

Merge commits on metadata refs create non-linear history. This is fine — the history of a metadata ref is an audit log, not a development narrative. DAG order with timestamps is sufficient for reconstruction.

### Push Timing

Three modes, all producing the same commits. The only variable is transport timing.

**Interactive (default).** Commit + push on every action. A comment, approval, or link is a collaborative signal — holding it locally defeats the purpose. The tooling runs `commit && push` as one operation.

**Daemon.** Commit immediately, push on a short debounce or periodic interval. Useful when rapidly commenting — five comments in thirty seconds, the daemon waits for a pause, then pushes once.

**Offline.** Commit locally, push on reconnect. `git forge sync` (or the daemon) pushes the metadata ref. The server auto-merges all accumulated commits against whatever happened while offline. One push, multiple operations.


## Reviews (Workflow)

### Creating a Review

A developer pushes a branch and runs `git forge review request`. This:

1. Generates a review ID (via the counter protocol).
2. Creates `refs/meta/reviews/<review-id>` with metadata pointing at the branch's commit range relative to the target branch.
3. Records revision 001 with the current head commit.

### Updating a Review

The developer pushes new commits or rebases. They run `git forge review update` (or the daemon detects the branch update). A new revision entry is added to the review ref.

### Reviewing

The reviewer opens the review. The tooling:

1. Reads the review ref to get the commit range.
2. Shows the diffs.
3. Comments land as `git-metadata` on blob oids (not on the review).
4. Approvals land as `git-metadata` on patch-ids, range patch-ids, blob oids, or tree oids (not on the review).

### Merging

The developer submits to the merge queue. The queue processor verifies policy, rebases, builds, and pushes. The review state transitions to `merged`.

### Stale Comments

There are no stale comments. A comment is on the code, not on the review. If the code changes, the comment is reanchored. If the code is deleted, the comment is orphaned. If the comment is no longer relevant, someone resolves it explicitly.


## Releases

### Releases as Workflows

A release is a repository maintenance workflow. It is not a build step. Given the commits since the last release, Forge determines the version bump, applies version changes, and generates the changelog.

`git forge release prepare`:

1. Parse conventional commits since the last tag.
2. Determine bump type (major, minor, patch).
3. Run version updaters (language-native tools).
4. Generate changelog.
5. Create a release branch with the changes.
6. Create a review ref for the release.

`git forge release publish`:

1. Tag the merged commit.
2. Attach build artifacts to the release ref.
3. Push.

### Release Refs

```
refs/tags/v1.2.0            → signed commit
refs/meta/releases/v1.2.0   → tree
├── meta                     # toml: version, date, author
├── changelog                # markdown
├── artifacts/
│   ├── x86_64-linux/
│   └── aarch64-darwin/
└── signatures/
```

### Automation Modes

Fully manual, semi-automated, or continuous. In semi-automated mode, every merge to main triggers `git forge release prepare`. If there are releasable commits since the last tag, a release branch and review appear. In continuous mode, preparation and publication happen automatically. The review requirement in policy.toml is the control point.


## Notifications

Notifications have two layers:

**Durable record.** Post-receive hooks write notification entries to a per-user namespace on new events (comment on your code, assignment, approval request). These are Git objects — the audit trail and offline fallback.

**Real-time delivery.** The HTTP server (the same layer that handles auth and ref filtering) pushes events to connected clients. Missed events are recovered from the durable record.

The daemon subscribes to the server and surfaces notifications in the editor via the LSP.


## Search and Indexing

Scanning all refs for queries ("all open issues," "all unresolved comments on this file") is acceptable at small scale and slow at large scale.

Derived indexes are daemon-maintained or built on demand:

- **Open issues index:** list of issue IDs with state=open. Updated on every issue ref mutation.
- **Comments-by-blob index:** the `git-metadata` fanout provides this directly.
- **Links indexes:** the relational metadata trees provide direct lookup in both directions.
- **Approvals-by-review index:** list of patch-ids approved within a review's commit range. Derived from the review's revisions and the approvals ref.

Indexes are convenience. The source of truth is always the refs. A missing or stale index is rebuilt from refs. Correctness never depends on the index.


## UI Server

The UI server has direct filesystem access to the repository. It does not go through the Git transport protocol.

**Entity creation.** ID assignment is a filesystem lock + increment. No CAS retry loop, no hooks. The UI is the fast path for entity creation.

**Read-optimized projections.** The data model is append-only refs and metadata. The UI maintains derived indexes in memory or on disk for fast querying:

- **Comment reanchoring visualization.** Blame-mapped comment threads inline on a file view.
- **Review rounds.** Diffing revision N against revision N-1 of a review. The revisions entries provide the commit pairs; the UI renders interdiffs.
- **Approval coverage.** Which patches in a review are approved and which aren't, aggregated into a progress view across all four approval levels.
- **Cross-reference graph.** The relational metadata trees provide both directions. The UI renders the full graph — "these 3 comments and 2 issues reference this commit."

Correctness never depends on the UI's indexes — they can always be rebuilt from refs.


## CLI

```
git forge
├── review
│   ├── request
│   ├── update
│   ├── list
│   ├── show <id>
│   ├── approve [<id>]
│   ├── comment <file>:<line-range> [-m message]
│   └── resolve <comment-id>
│
├── issue
│   ├── create <title>
│   ├── list
│   ├── show <id>
│   ├── close <id>
│   ├── reopen <id>
│   ├── comment <id> [-m message]
│   ├── assign <id> <user>
│   └── label <id> <label>
│
├── check
│   ├── run <name>
│   ├── status <commit>
│   └── list
│
├── release
│   ├── prepare
│   ├── publish
│   ├── list
│   └── show <tag>
│
├── queue
│   ├── create <name>
│   ├── push <name> <ref>
│   ├── pop <name>
│   ├── list <name>
│   └── peek <name>
│
├── contributor
│   ├── add <key-file> [--role=<role>]
│   ├── list
│   ├── remove <fingerprint>
│   └── role <fingerprint> <role>
│
├── secret
│   ├── set <name> [--value=... | --file=...]
│   ├── list
│   ├── grant <name> --runner=<fingerprint>
│   └── revoke <name> --runner=<fingerprint>
│
├── sync
│   ├── github --repo=<org/project> --import
│   └── ...
│
└── config
    ├── show
    └── edit
```

All list commands support `--json`. Every subcommand maps to a ref operation (except `secret`, which is a server API call).

`git forge review comment` is sugar for `git metadata add` on the appropriate blob oid with the comment tree structure. `git forge review approve` is sugar for `git metadata add` on the patch-ids and range patch-id in the review's range.


## Server

The minimal Forge server is an HTTP middleware in front of `git-http-backend`. It handles:

1. **Authentication.** Verifies the user's identity.
2. **Ref filtering.** Strips private refs from the advertisement based on role.
3. **Pre-receive hooks.** Enforces policy, signatures, approvals, path permissions, and check results.
4. **Post-receive hooks.** Triggers reanchoring, notification writes, index updates, inbox canonicalization.
5. **Real-time notifications.** Pushes events to connected clients.
6. **Merge queue processing.** Rebase, build, push.
7. **Metadata auto-merge.** Three-way tree merge on non-fast-forward metadata pushes.
8. **Secret management.** Encrypted storage, ACL enforcement, tmpfs injection for runners.

The server is stateless (except for the secret store). All other state is in Git. If the server goes away, the data is intact. The server is a convenience, not a dependency.


## Migration

### GitHub App

A GitHub App bridges adoption. It receives webhook events and writes refs via GitHub's API:

- Mirror issues and comments into Forge ref format.
- Update review refs on PR events.
- Write approval refs on PR review events.

Everything written as refs is portable. When a team migrates off GitHub, the refs come with them.

### Sync

```sh
git forge sync github --repo=org/project --import
```

Issues, PRs, comments, labels, and milestones are read via the platform API and written as Forge refs. Sync state is stored as a ref.
