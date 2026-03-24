+++
title = "Kiln: A Content-Addressed Build Engine Backed by Git"
subtitle = "Design Specification"
version = "0.1.0"
date = 2026-03-11
status = "Draft"
summary = """
Kiln is a build and execution engine defined as an open spec with a reference \
implementation backed by Git's content-addressed object store. It treats build \
actions as pure functions of content-addressed inputs, providing hermetic \
caching, signed outputs, and graduated isolation — from no enforcement to full \
sandboxing."""
+++

# Kiln: A Content-Addressed Build Engine Backed by Git


## Foundation

### Content-Addressed Storage as Build Primitive

Containers achieve hermetic builds by isolating the environment.
Content-addressed storage achieves hermeticity by making all inputs explicit and immutable by content hash.
These are fundamentally different strategies for the same goal: ensuring builds are a pure function of their inputs.

Every build input — source files, toolchains, libraries, even the compiler binary itself — is stored and referenced by its content hash.
A build rule becomes a function: `hash(output) = f(hash(input₁), hash(input₂), ..., hash(toolchain))`.
If all input hashes match a prior invocation, the executor skips execution entirely and retrieves the cached output from the store.

Where containers provide filesystem isolation, content-addressed storage replaces it with explicitly declared inputs identified by hash.
There is nothing ambient to leak because the build system only provides content-addressed artifacts to the build action.
Where containers provide reproducible base images, content-addressed storage replaces them with content-addressed toolchains.
The compiler isn't "whatever's in the container" — it is a specific hash.
Network isolation is still needed and is typically provided by lightweight kernel-level mechanisms rather than full containers.

This approach is strictly better than containers in several dimensions.
Caching is granular at the individual artifact level rather than the coarse layer level of container rebuilds.
Artifacts are shared across unrelated builds automatically when they share the same toolchain hash.
And determinism is structural rather than conventional — a Dockerfile can still contain `apt-get update` nondeterminism, while a content-addressed system forbids undeclared inputs by design.

### Git as the Store

Git is already a content-addressed store.
Its object database stores blobs, trees, and commits identified by SHA hashes.
A tree object is a content-addressed filesystem snapshot.
A commit is an immutable record pointing at a tree.
The transport protocol supports fetching individual objects by hash.

With partial clone and promisor remotes, Git becomes a content-addressed blob store with a well-defined transport protocol.
A tool can fetch blobs on demand by hash, requesting only the specific trees and blobs needed.
The concern that Git "downloads everything" disappears under this model.
And for binary artifacts, the compression concern is overstated — most package managers store compressed tarballs anyway and are not performing cross-version delta compression either.

Git's Merkle tree structure provides deduplication and change detection for free.
Changing one file changes the root tree hash, which changes the action identity, which invalidates exactly the right caches.
Shared source files across builds are stored once.
The object database is already the storage layer.

### Git Object Store Tuning

Binary build artifacts — `.so`, `.a`, `.rlib`, `.dylib`, `.dll` — do not compress well with zlib and produce negligible savings from cross-version delta compression.
Two knobs control Git's behavior for these files.

Loose object compression (`core.looseCompression`) controls zlib level during `git add` and `git_odb_write`.
The default level 6 is the bottleneck when ingesting large `target/` directories.
Level 0 produces a valid zlib stream (stored blocks, no deflation) and cuts write time dramatically.
Level 1 provides a modest compression ratio at minimal CPU cost.
For a build system that ingests artifacts transiently before the next repack, level 0 or 1 is the right tradeoff.

Delta compression (`.gitattributes -delta`) controls whether `git gc` and `git repack` attempt binary diffs between versions of an object.
For compiled artifacts, delta compression is nearly always a waste of CPU for negligible size savings:

```gitattributes
*.so -delta
*.a -delta
*.rlib -delta
*.dylib -delta
*.dll -delta
*.rmeta -delta
```

With libgit2 (`git-kiln`'s likely implementation path), these knobs are per-blob.
`git_odb_backend_loose()` accepts a `compression_level` parameter.
The adapter can write source blobs at default compression and binary artifact blobs at level 0, in the same repository, without a repo-wide config change.
`git_odb_write()` dispatches to the backend — the choice of compression level is a storage-layer concern, invisible to refs, trees, gc, fsck, and every other layer.
The SHA is computed from logical content, not compressed representation.

### Actions as Pure Functions

The core abstraction is that build actions are pure functions.
An action takes a set of content-addressed inputs and produces a content-addressed output.
Same inputs always produce the same output.
There are no side effects — no network access, no reading undeclared files, no timestamps.
All inputs are enumerated and content-addressed.

The content-addressed store is the mechanism that enforces and exploits this property.
Content-addressing inputs makes the function's domain explicit.
Content-addressing outputs makes results memoizable.
The entire system follows from taking "builds are pure functions" seriously and then asking what infrastructure is needed to guarantee and leverage that property.


## Architecture

Kiln is defined as an open spec and a Git-backed reference implementation called `git-kiln`.
Language adapter authors interact with kiln directly.
The spec defines content-addressed trees, actions, and cache lookups.
The reference implementation backs the spec with Git's object store.
Another implementation could use an OCI registry or a custom CAS.
Language adapters target the kiln spec, not `git-kiln`, keeping the adapter ecosystem portable.

The architecture has three layers.
Layer zero is Git itself — refs, objects, trees, blobs, and signing.
Layer one is Kiln, handling plans, actions, caching, and sandboxing, which is language-aware.
Layer two contains language adapters like `cargo-kiln` and `uv-kiln`, which are language-specific.

Tools built on top of Kiln — CI systems, collaboration platforms, release automation — consume its outputs (signed build refs, cached artifacts, action results) without Kiln knowing or caring about them.
Kiln is a build engine, not a workflow system.


## The Plan Tree

A language tool receives a worktree and emits a plan tree — a set of nodes describing build steps.
The plan tree is stored in Git's object database, not in the working tree.
No kiln-specific files clutter the source.

The plan has one node type: actions.
Actions are pure.
They read declared inputs and produce outputs without modifying the source tree.
The worktree is always immutable from Kiln's perspective.
A build compilation is an action.
A test suite is an action.
Codegen is an action — it reads source like a protobuf definition and produces an output tree containing generated files like a Rust module.
Actions are cached by hashing their inputs, and their outputs are stored and reused across machines.

Actions can depend on other actions.
Codegen before build — the build action declares the codegen action's output as an input.
The planner emits the full DAG.
Kiln executes it in dependency order.

Operations that mutate the source tree — version bumping, changelog generation, formatting, license header insertion — are not build steps.
They are repository maintenance workflows handled outside Kiln.
These workflows make commits.
Kiln then sees those commits as ordinary source.

Each node in the plan has a small fixed schema:

```toml
name        # cache namespace (language-native package name by default)
toolchain   # content-addressed tree ref (optional, defaults to planner's)
inputs      # list of paths, hashed as trees
deps        # dependency map (see below)
env         # map of environment variables, part of cache key
run         # list of command + arguments (exec-style, no shell interpretation)
outputs     # named output map: key → path (artifact outputs, signed and shared)
expects     # expected output hashes: key → hash (optional, see Verified Fetch)
state       # named state map: key → path (convenience cache, not signed)
profiles    # list of profiles this node participates in (optional, default: all)
```

The format is intentionally minimal.
There are no conditionals, no templating, no variable substitution, no inheritance.
Platform variants are separate nodes.
If two nodes share configuration, the language adapter deduplicates when generating them.
The plan format is a serialization boundary, not a programming language.
All intelligence lives in the language adapter that generates plans and in kiln's DAG executor that evaluates them.

### Profile Overrides

Profile-specific overrides for `env` and `run` are declared inline on the node:

```toml
[[action]]
name = "mylib"
inputs = ["crates/mylib/"]
run = ["cargo", "build", "-p", "mylib"]

[env]
RUSTFLAGS = ""

[env.profile.release]
RUSTFLAGS = "-C opt-level=3"

[run.profile.release]
run = ["cargo", "build", "-p", "mylib", "--release"]
```

One plan, one planning action, no re-planning on profile switch.
Kiln resolves overrides at execution time.
The cache key for each action includes the resolved profile-specific fields, so debug and release outputs cache independently.
The planner only re-runs when the worktree or planner binary changes — not when the developer switches between `--profile=debug` and `--profile=release`.
This avoids the Bazel analysis-cache problem, where changing a configuration flag discards the entire analysis graph and forces expensive re-planning.

For rare cases where a profile requires a structurally different DAG — PGO workflows (instrumented build → benchmark → rebuild with profile data), cross-compilation with proc macros — the planner can emit a profile-specific plan keyed as `plan/<worktree_hash>-<profile>`.
Kiln checks for a profile-specific plan first, falls back to the shared plan with profile selection.
Common profiles pay no re-planning cost.
Exotic profiles pay it only when necessary.

The `profiles` field is a whitelist.
Omit it and the node runs in all profiles.
Include it and kiln skips the node for unlisted profiles:

```toml
[[action]]
name = "mylib-test"
inputs = ["crates/mylib/"]
profiles = ["debug", "ci"]
run = ["cargo", "test", "-p", "mylib"]
```

This covers the common cases: tests run in debug and CI but are skipped in release.
LTO runs in release but is skipped in debug.
Benchmarks run only in a `bench` profile.
The DAG is the same object; the profile selects a subgraph.
Skipped nodes aren't cache-invalidated because they never ran — switching from debug to release doesn't touch test cache entries, and they're still warm when you switch back.

### Toolchain Field

The `toolchain` field is a content-addressed tree ref, not a container image.
It declares what tools the action needs, not how to run them.
When Docker is the executor, kiln loads the tree into a container.
When using `unshare`, kiln mounts the tree directly.
The executor decides the mechanism.
Most nodes omit this field and inherit the toolchain from the planner's environment.

### Run Field

The `run` field is an argv list — `["cargo", "build", "--release"]`, not a shell string.
No shell interpretation, no quoting ambiguity, no implicit `/bin/sh -c`.
The executor execs the command directly.
If a node genuinely needs shell features (pipes, redirects, globbing), the command is `["sh", "-c", "..."]` and the shell dependency is explicit.
This matters at higher isolation levels where the shell itself may not be in the sandbox unless declared.

### Outputs and State

The `outputs` field is a named map of artifact outputs — hermetic build products that are signed, shared, and cached across machines.
Each key names a public output; the value is the path within the action's capture directory:

```toml
[outputs]
lib = "target/release/libfoo.so"
bin = "target/release/foo"
```

The `expects` field is a map of expected content hashes for artifact outputs.
It enables verified fetch actions — see the Verified Fetch section.

The `state` field is a named map of state outputs — convenience caches for language tooling that are materialized to the worktree but never signed or shared.
These are the `target/` and `.venv/` directories that LSPs and editors need:

```toml
[state]
target = "target/"
```

The distinction is structural.
Artifact outputs go into the object store, get signed by CI, and are fetchable by any consumer.
State outputs are local-only convenience — restored to the worktree after a build so that `rust-analyzer` sees `target/` and `pyright` sees `.venv/`, but never shared across machines.
If state is stale or missing, the language tool rebuilds incrementally.
No correctness depends on state.

### Dependencies Between Actions

Consuming another node's outputs uses the dependency map:

```toml
[deps.foo]
lib = "myservice/native.so"
```

The left side of a dep entry is an output key from the producing node.
The right side is where it lands in the consuming node's input tree.
Output keys are the interface contract — internal build artifacts that shouldn't leak across boundaries simply aren't named.
To consume the entire output tree:

```toml
[deps.foo]
tree = "vendor/foo/"
```

Node names default to the language's native package names.
A Rust crate named `foo` produces a node named `foo`.
A Python package named `myservice` produces a node named `myservice`.
Users already know these names.
This makes nodes referenceable without requiring knowledge of planner internals.

### Per-Key Dependency Hashing

When a node depends on another node's output, its action hash includes the content hash of the *specific output key* it references — not the root tree hash of the producing node's commit.
Git's Merkle tree structure provides this naturally: every subtree and blob within a tree object has its own hash.

This means a node's cache is only invalidated when the specific outputs it consumes change.
If node A produces outputs `lib` and `metadata`, and node B depends only on `A.lib`, a rebuild of A that changes `metadata` but produces a bit-identical `lib` does not invalidate B's cache.

This property is essential for cross-language composition.
A Python service that depends on a Rust crate's `.so` file should not be invalidated when the Rust crate's `.rmeta` metadata changes.
It is also essential for OCI image assembly, where a deployment image may consume specific facets of multiple build nodes — a binary from one, a shared library from another, a config tree from a third.
Each facet's hash enters the image node's action hash independently.

The commit parent links on the build graph still point at the producing node's commit (the whole action), preserving coarse-grained provenance.
Per-key hashing is a cache-key concern, not a provenance concern.
Provenance answers "what action produced the inputs to this build."
Cache validity answers "did the specific bytes I depend on change."
These are different questions answered by different mechanisms.


## Keeping the Working Tree Clean

All kiln state lives in the Git object database under a custom ref namespace.
The working tree stays pure source.
`git status` is clean.
`git branch` and `git log` show nothing kiln-related.

### Build DAG and Cache Index

The ref namespace separates two concerns: the build DAG (the record of truth) and the cache index (a derived acceleration structure).

**Build DAG — per-target refs (ledger):**

```text
refs/kiln/plans/<worktree_hash>                       → plan tree
refs/kiln/outputs/<name>/<profile>/<arch>             → signed commit (tree = build output)
```

Each target gets its own ref.
Commits on that ref are builds of that target — the commit history is the build history.
A commit's tree is the build output.
A commit's additional parents link to the output commits of its dependencies, encoding the provenance DAG in Git's native structure.
`git log refs/kiln/outputs/mylib/release/x86_64-linux` shows every build of mylib.
`git log` with parent traversal shows the full dependency graph.

Two CI jobs building different targets write to different refs — no contention.
Two CI jobs building the same target race on the same ref, but both produce the same output tree hash, so last-write-wins is correct.

Plans are keyed by worktree hash only (not per-profile) since plans are profile-agnostic — profile selection happens at execution time.

**Cache index — metadata ref:**

```text
refs/kiln/cache → commit → tree
  <hash-prefix>/
    <action-hash>/
      commit-oid          # blob: "<target-ref>:<commit-oid>"
```

The cache index maps action hashes to commit OIDs using two-level fanout (the git-metadata pattern for hash keys).
Cache lookup is: compute action hash, look up in the index, fetch the commit.
Multiple CI jobs writing disjoint action hashes touch disjoint tree paths and auto-merge via three-way tree merge.

The cache index is derived, not authoritative.
If it is lost or stale, it can be rebuilt by walking all `refs/kiln/outputs/` refs and extracting action hashes from commit messages.
Correctness never depends on the index — it exists for fast lookup.
This is the same pattern as the sequential counter optimization in git-ledger: a performance structure, not a correctness requirement.

**Why separate them:** The build DAG is the thing you `git log`, `git verify-commit`, and `git diff` against.
The cache index is the thing the executor queries during bottom-up DAG traversal.
One is a record of what was built, by whom, from what.
The other is an optimization for "does this build exist."
Conflating them (as a flat `refs/kiln/outputs/<action_hash>` scheme would) makes cache lookup O(1) but loses build history, target identity, and DAG structure.
Separating them preserves all three while keeping cache lookup fast via the metadata index.

Alternative cache indexes — a local-only one for unsigned dev builds, a CI-signed one for shared builds — can point into the same set of target refs.
The DAG doesn't care how you found it.

`git clone` doesn't fetch build refs because the default refspec only includes `refs/heads/*` and `refs/tags/*`.
Build objects share deduplication with source blobs.
And `git gc` handles eviction naturally — dropping a build ref makes its objects unreachable and they get pruned.

Build tools that need artifacts explicitly fetch them:

```sh
git fetch origin refs/kiln/outputs/mylib/release/x86_64-linux
```


## Output Modes

An action's output has two parts: artifact outputs (signed, shared) and state outputs (local convenience).
Kiln provides three modes for consuming these:

**Default materialization.**
After a successful build, kiln restores each final target's state outputs to the worktree at the language's conventional paths — `target/` for Rust, `.venv/` for Python.
This is what `cargo build` and `uv sync` do today.
LSPs, editors, and language tooling see a normal project.
Hermeticity is enforced during the build inside the sandbox.
State restoration happens after.
These are completely independent.
This is a major advantage over Bazel, which breaks LSP integration by imposing its own output layout.

**`kiln materialize <action> [--output=key]`.**
Explicitly copy an action's artifact outputs into the worktree.
This is for codegen and similar cases where generated files should be visible to editors, grep, and other tools on disk.
The files appear as untracked (the user decides whether to `.gitignore` or commit them).
Kiln doesn't care — the write is a user-initiated side effect after the pure action completes.
Without `--output`, all artifact outputs are materialized.
With it, only the named output is materialized.

**`kiln enter <action> [--isolation=N]`.**
Present a transient merged view of the worktree and the action's output tree.
On exit, the worktree is untouched.
The implementation mechanism is platform-dependent (see Platform Support).
Isolation levels slot in naturally since transient contexts already operate in a namespace on Linux.
This is the primary interface for environments: `kiln enter env` overlays a toolchain tree onto the worktree, giving the developer a shell with the right tools.


## State Restoration for Language Tooling

State restoration is the mechanism that makes LSPs fast after a kiln build.
The restored files must be exactly what the language's tooling expects.

For Rust, `rust-analyzer` invokes `rustc --emit=metadata` on every crate for type checking.
Without restored state, this recompiles every dependency from scratch — minutes on a large workspace.
With restored state, it skips all cached deps and only checks the crate the developer is editing.

The minimal state set for Rust is:

- `target/<profile>/deps/*.rmeta` — type metadata for all crates (what rust-analyzer consumes)
- `target/<profile>/deps/*.rlib` — compiled crate archives
- `target/<profile>/build/*/out/` — build script outputs (generated code, cfg flags)

Rust-analyzer expects a debug profile build.
`cargo-kiln` should always emit a debug build action so that LSP integration works out of the box.
If the developer only requested a release build, the debug `.rmeta` files are still needed for the editor experience.


## Isolation Levels

Hermeticity is a dial, not a switch.
Kiln defines five isolation levels, each a superset of the previous.

Level zero provides declared inputs with no enforcement.
The build runs on the host.
Kiln hashes declared inputs for cache keys.
If the build reads undeclared files, kiln doesn't know.
Caching works but is only as correct as the declarations.

Level one provides read-only inputs.
The build runs on the host but the input tree is mounted read-only.
Writes go to a capture directory.
The build can still see the host filesystem and network, but it cannot mutate inputs.

Level two provides filesystem isolation.
The build runs in a mount namespace.
Only declared inputs are visible.
The host filesystem is gone.
Environment variables are controlled.
The build can still access the network and see the real clock.

Level three adds network isolation.
No network access at all.
If the build needs to download something, it fails.
All dependencies must be in the input tree.
This is where builds become truly reproducible.

Level four provides a full sandbox.
Fixed timestamps, PID namespace, no access to host information through `/proc`, deterministic file ordering.
The build sees a completely synthetic environment.

The plan declares the minimum supported level.
Kiln can enforce higher than declared but never lower.
CI can require level three as a minimum while local development defaults to level zero.
The same plan, the same rules, different enforcement.
This eliminates the need for separate development and release build modes.

When a developer tightens isolation, the system tells them exactly what breaks:

```text
$ kiln build --isolation=2 crates/cli/
ERROR: action reads /etc/ssl/certs (undeclared input)
```

Each node can also specify its own isolation level as a field, allowing fine-grained control within a single build.


## Verified Fetch

Build actions at isolation level three and above cannot access the network.
But dependency fetching is inherently a network operation.
This creates a tension: how does a project with a level-three policy acquire external dependencies?

The answer is that fetch actions run at isolation level zero but declare expected output hashes.
Verification replaces isolation as the trust mechanism.

Lock files already contain content hashes for every dependency.
`Cargo.lock` has them.
`uv.lock` has them.
`go.sum` has them.
`package-lock.json` has them.
The planner reads these hashes and declares them on each fetch action's `expects` field.

The enforcement rule becomes: `enforce(max(declared, policy))` unless the action has expected output hashes, in which case the declared level is used and verification replaces isolation as the trust mechanism.
This is not an exception to the rule — it is a stronger guarantee.
A hash-verified output from level zero is more trustworthy than an unverified output from level three, because verification is a proof and isolation is a precaution.

The expected hashes are not authored or managed by the developer.
They are derived mechanically from the lock file by the planner.
CI runs the same planner, reads the same lock file, derives the same hashes, and verifies independently.
The developer's workflow is unchanged: `cargo update` modifies `Cargo.lock`, the next `kiln build` re-plans (because the lock file is a planner input), emits new expected hashes, and the vendor action runs with a new cache key.

Build actions downstream of verified fetch nodes run at the policy-required isolation level.
The fetched output is their input.
Only the fetch itself is exempted from network isolation, and only because verification provides a stronger correctness guarantee than isolation alone.

Cache hits are also verified.
If a cached fetch output exists but its hash doesn't match the expected value, kiln treats it as a miss and re-fetches.
Cache poisoning is caught.

### Per-Crate Vendor Nodes

Fetch actions are emitted at per-crate (or per-package) granularity, not as a single monolithic vendor action per lock file.
The planner reads the lock file and emits one fetch node per dependency:

```toml
[[action]]
name = "vendor-serde"
isolation = 0
inputs = ["Cargo.lock"]
run = ["cargo-kiln-fetch", "--crate=serde", "--version=1.0.210"]

[outputs]
src = "vendor/serde/"

[expects]
src = "sha256:ab34ef..."
```

This granularity is critical for cache invalidation.
If fetch actions are coarse-grained — one action producing the entire vendor tree — then bumping a single dependency changes the vendor output tree hash, which enters every downstream build node's action hash via per-key dep hashing, invalidating the entire build cache.
Per-crate vendor nodes confine invalidation: bumping serde changes only `vendor-serde`'s output, and only crates that depend on serde recompute their action hashes.

The implementation can batch the actual network fetch (one `cargo vendor --locked` invocation) and split the output directory into per-crate trees.
The per-node granularity is a cache-key concern, not necessarily a network-operation concern.
Git deduplicates unchanged crate source blobs across vendor runs at the blob level.

Downstream build nodes depend on specific vendor crates by name:

```toml
[[action]]
name = "mylib"
isolation = 3
inputs = ["crates/mylib/src/"]

[deps.vendor-serde]
src = "vendor/serde/"

[deps.vendor-tokio]
src = "vendor/tokio/"

run = [
  "rustc", "--edition=2021", "--crate-type=lib", "--crate-name=mylib",
  "crates/mylib/src/lib.rs",
  "--extern", "serde=vendor/serde/libserde.rlib",
  "--extern", "tokio=vendor/tokio/libtokio.rlib",
  "--out-dir", "out/",
]

[outputs]
lib = "out/libmylib.rlib"
```

The `rustc` invocations come from cargo's build plan — cargo is used for planning, not execution.
At isolation level three, the network is blocked; all inputs are declared in the plan.

The same pattern applies to every language with a lock file: `uv-kiln` emits per-package fetch nodes reading `uv.lock`, `go-kiln` emits per-module fetch nodes reading `go.sum`.


## Platform Support

Isolation levels zero and one work everywhere — they require only filesystem permissions and directory structure, no kernel features.
Level zero is pure convention.
Level one uses read-only mounts on Linux and read-only directory permissions on macOS (weaker enforcement but catches accidental writes).

Levels two through four require Linux kernel features: mount namespaces (`unshare`), network namespaces, PID namespaces, `seccomp`.
These are not available on macOS or Windows.
On non-Linux platforms, kiln enforces up to level one natively.
For higher levels, kiln delegates to a Linux VM — Docker Desktop on macOS, WSL2 on Windows.
The build runs inside the VM with full namespace isolation.
The overhead is the VM boundary, but correctness is preserved.

This is an honest tradeoff.
Most developers work at level zero locally.
CI runs on Linux at level two or three.
The rare developer who wants local level-two enforcement on macOS pays the VM cost.
The common case is fast; the strict case is correct.

`kiln enter` on macOS at level zero uses a tmpdir copy or symlink farm — heavier than overlayfs but functional.
At higher isolation levels on macOS, it enters a Linux VM context.


## The Executor

The executor evaluates a plan through a bootstrap sequence that bottoms out at a single hardcoded action.
The bootstrap has two phases: setup and execution.

**Setup** (outside the action system):

1. Read `env.toml` (file read, not an action).
2. Fetch planner and toolchain trees from the package store (git fetch, not an action).

These two steps are necessarily outside the action system.
You cannot execute an action to fetch the tool that creates actions.
`env.toml` is a fixed-format file that kiln reads directly.
Git fetch is a transport operation.
Neither benefits from being modeled as actions.

**Execution** (everything is an action from here):

3. Require a clean worktree (the worktree hash is HEAD's tree, making it deterministic).
4. Hash the worktree tree.
5. Construct the planning action — the single hardcoded action kiln knows how to build without a planner.
   Its inputs are the worktree and the planner binary from `env.toml`.
   Its command is the planner invocation.
   Its output is the plan tree.
   The profile is not part of the planning action's cache key — plans are profile-agnostic.
6. Check the cache for the planning action.
   On a hit, retrieve the cached plan.
   On a miss, execute the planner and store the plan.
   The cache key is `hash(worktree, planner_binary, env.toml)`.
7. Read `kiln.toml` files (root-level and subdirectory).
   Graft extra actions and edges onto the plan.
8. Select the active profile.
   Resolve per-node profile overrides for `env` and `run`.
   Skip nodes whose `profiles` whitelist excludes the active profile.
9. Traverse the full plan DAG bottom-up.
   For each active node, compute the action hash.
   The action hash includes: the node's resolved `env` and `run` fields, the hashes of its `inputs`, and — critically — the content hashes of the *specific output keys* it references from its dependencies, not the root tree hashes of those dependencies' commits.
   This per-key hashing means a dependency that rebuilds but produces bit-identical outputs at the referenced keys does not invalidate its dependents.
   Look up the action hash in the cache index (`refs/kiln/cache`).
   On a hit, retrieve the cached commit and its output tree.
   On a miss, build.
   For nodes with `expects`, verify output hashes after execution.
10. Store artifact outputs as commits on per-target refs (`refs/kiln/outputs/<n>/<profile>/<arch>`).
    The commit's tree is the build output.
    The first parent is the previous build of that target.
    Additional parents are the output commits of the node's dependencies.
    Update the cache index with the new action hash → commit OID mapping.
11. Materialize state outputs to conventional paths for final build targets.

The planning action is cacheable like any other.
Same worktree and same planner version produce the same plan.
The planner doesn't even run on a cache hit.
Switching profiles reuses the cached plan and only recomputes action-level cache keys with resolved overrides.

For dirty worktrees, kiln rejects by default.
A `--allow-dirty` flag hashes the actual working tree instead of HEAD's tree.
The cache key includes uncommitted changes, so results are correct but only locally useful — they cannot be signed or shared because they don't correspond to a commit.



## Action Failure

When an action exits with a non-zero status, kiln does not cache the output.
The action is considered failed.
Partial outputs are discarded.
Downstream dependents do not run.

This means flaky tests are never cached as failures.
Re-running the build re-executes the failed action.
If it passes, the output is cached normally.

However, the build work preceding a failed test is still cached.
If action A (compilation) succeeds and action B (tests) fails, A's output is in the cache.
Re-running only re-executes B.

For reporting, failed actions produce a structured error ref containing the exit code, stderr capture, and the action's identity hash.
The error ref is not a cached output — it is a record of what happened, not a reusable result.

A `--cache-failures` flag exists for specific use cases like expensive test suites where a known failure should not be re-executed on every invocation.
When enabled, the cached failure is returned immediately with its original error output.
This is opt-in and never the default.


## Bootstrapping With Docker

The executor needs a sandbox.
Docker is a reasonable starting point.
The bootstrapping ladder has clear stages.

At stage zero, Docker is the executor.
The toolchain is a Docker container.
The executor accepts the impurity of Docker's runtime but gets the caching model working.
At stage one, the toolchain becomes a tree in the Git object store.
Docker provides only the namespace and sandbox — a dumb isolation shell around a content-addressed filesystem.
At stage two, Docker is replaced with lighter isolation using `unshare`, `pivot_root`, and `seccomp`.
There is no container runtime dependency.
Stage three, which is optional and Nix-like, builds the toolchain itself through the system, turtles all the way down to a bootstrap binary.

For most use cases, stage one is where the cost-benefit ratio peaks.
Content-addressed caching and explicit inputs are achieved while Docker handles the boring isolation work.


## Caching

The simplest useful form of kiln is a cache.
Existing lock files are already content-addressed dependency specifications.
`Cargo.lock`, `uv.lock`, `go.sum`, and `package-lock.json` contain content hashes of every dependency.
Hash the lock file plus the source tree plus the toolchain, and you have a cache key.
Hit means return the stored output.
Miss means build and store the result.

The Git remote is the cache.
CI runs `kiln build`, populates the cache, pushes build refs.
A developer clones and runs `kiln build`.
Every external crate, every pinned dependency, every unchanged module is an instant cache hit fetched from the remote.
No sccache, no S3 bucket, no cache key heuristics.

### Cache Key Refinement

Rustc tracks every file it opens during compilation and writes the results to `.d` files when `--emit=dep-info` is passed.
These files list the actual source files read, providing finer-grained dependency information than directory-level hashing.

On a debug build, `.d` files are produced as part of the output.
A release build can use that information for refined cache keys.
Since both builds are happening anyway, there is no extra pass and no performance cost.
The refinement can also be triggered explicitly with an optimization flag for CI environments that want tighter cache keys.

Refinement data is stored per-node in the index, decoupled from the worktree hash.
When a new commit arrives, kiln checks whether a refinement exists for a given node.
If so, it hashes only the files that matter instead of the whole directory.
If a refined cache lookup produces a false hit — because a new import added a file the refinement didn't know about — the build proceeds normally, captures a new `.d` file, and updates the refinement.
The refinement is optimistic with a correctness backstop.


## Profiles

Builds support profiles.
The profile is passed to kiln at invocation time:

```sh
kiln build --profile=debug
kiln build --profile=release
```

Kiln resolves the profile by selecting the subgraph of nodes whose `profiles` whitelist includes the active profile (or all nodes if `profiles` is omitted), then applying per-node overrides from `env.profile.<name>` and `run.profile.<name>`.
The plan itself is not regenerated — the same cached plan serves all profiles.

Different profiles produce different resolved `env` and `run` fields, and therefore different action hashes and cache keys.
Debug and release outputs cache independently.
Switching profiles is free at the planning level; only actions whose resolved fields change produce new cache keys.


## Container Environments as Actions

A container environment is itself a build artifact.
A Dockerfile is an input; the output is a filesystem tree:

```toml
[[action]]
name = "runtime"
toolchain = "git://kiln-packages/docker@27"
inputs = ["Dockerfile"]
run = ["docker", "build", "-t", "scratch", "."]

[action.outputs]
rootfs = "rootfs/"
```

The output tree is a content-addressed filesystem.
Other actions use it as their `toolchain`.
`kiln enter runtime` drops the developer into it.
The container image isn't special infrastructure — it's a cached, content-addressed build artifact like everything else.
Change the Dockerfile, the action hash changes, the environment rebuilds.
Don't change it, cache hit.

This unifies the development path.
`kiln enter runtime` is the same operation whether the tree came from a Dockerfile, an import, or a from-scratch kiln build.
The entry mechanism changes (Docker → unshare → chroot), the abstraction doesn't.
Teams get reproducible Docker-based environments through kiln's caching before any native isolation exists.

The one honest impurity: the action that builds the container environment needs a host Docker daemon.
That's the bootstrap dependency acknowledged at stage zero — Docker is the one ambient input accepted until the system can self-host the sandbox.


## Deployment Images

Deployment images are compositions of build output trees assembled into a root filesystem:

```text
deployment_tree = merge(
    toolchain_output_tree,
    app_output_tree,
    config_tree
)
```

The result is a Git tree representing a complete filesystem.
It can be materialized and run directly with `unshare` and `chroot` — no Docker daemon, no image layers, no registry pull.

Deduplication is structural.
If two deployment images share OpenSSL, they share the same output tree hash because they were built from the same inputs.
Deduplication happens at the file level, not the layer level.
There is no Dockerfile — the image is a composition of verified build outputs.
Incremental image updates swap a single entry in the tree when one component changes.

For compatibility with existing infrastructure, a `kiln export --oci` command serializes the tree as a standard OCI image pushable to any registry.
The internal representation is Git trees.
The export format is whatever the deployment target needs.
Docker, Kubernetes, ECS, and Cloud Run all accept the result without knowing it came from a Git tree.


## Ephemeral Inputs

Not all action inputs are content-addressed.
Secrets — API keys, deploy tokens, signing credentials — must be available during execution but cannot be stored in the object database, included in cache keys, or appear in signed outputs.
Git repos get cloned, forked, mirrored.
A secret in a ref is a secret on every machine that fetches.

Kiln models these as ephemeral inputs.
They are declared in the action node but handled differently from regular inputs:

```toml
[[action]]
name = "deploy"
inputs = ["target/release/myapp"]
run = ["./scripts/deploy.sh"]

[ephemeral]
AWS_ACCESS_KEY = { type = "file", mount = "/run/secrets/aws" }
DEPLOY_TOKEN = { type = "file", mount = "/run/secrets/deploy" }
```

Properties of ephemeral inputs:

- **Declared but not hashed.**
  The action node names which ephemeral inputs it requires.
  The names are part of the action definition (and therefore visible in review), but their values are excluded from the action hash and cache key.
  Two runs with different secret values but identical content-addressed inputs produce the same cache key.
  This is correct — the secret enables a side effect (deployment, signing), not a deterministic build output.
- **Never cached.**
  Ephemeral inputs are not stored in the object database.
  They exist only for the duration of the action's execution.
- **Injected by the executor, not the runner.**
  The executor (or whatever host system manages the action's lifecycle) provides ephemeral values.
  A compromised action cannot request secrets it didn't declare.
  The host verifies that the declared ephemeral names are authorized for the runner's identity before injection.
- **Mounted, not exported.**
  At isolation level two and above, ephemeral inputs are written to a tmpfs volume mounted read-only into the sandbox (e.g., `/run/secrets/<name>`). tmpfs is memory-backed — never hits disk.
  No environment variable exposure, no `/proc/<pid>/environ` leakage, no child process inheritance, no accidental logging.
  At isolation levels zero and one, the executor writes to a temporary directory and deletes it after execution.
  Weaker guarantee, but the common accidental-leak vectors are still covered.
- **Destroyed on exit.**
  The tmpfs mount is torn down when the action exits.
  The temporary directory is deleted.
  No remnant.

Ephemeral inputs are an honest exception to the "everything is content-addressed" principle.
The alternative — storing secrets in Git — is worse in every dimension.
The design contains the exception: ephemeral inputs are declared (reviewable), excluded from the cache (no leakage into outputs), and scoped to execution (no persistence).

### Authorization

The executor must decide which ephemeral inputs a given runner is allowed to receive.
This is outside Kiln's scope — Kiln defines the declaration format and the injection contract, but the secret store and ACL are the host system's responsibility.

A typical integration: the host maintains an encrypted secret store keyed by name.
An ACL maps secret names to authorized runner identities (signing key fingerprints).
When Kiln's executor prepares to run an action with ephemeral inputs, it authenticates the runner, checks the ACL for each declared name, retrieves the values, and mounts them.
If any name is unauthorized, the action is rejected before execution.

### Audit

Every ephemeral input access — which secret, which runner, which action, when — should be logged by the host.
Kiln does not define the audit format, but it provides the action identity hash and runner identity to the host at injection time, giving the host everything it needs for a complete audit trail.


## Language Adapters

### The Adapter Contract

A language adapter is a command that reads a project and emits a plan tree.
That is its entire responsibility.
It does not fetch dependencies, orchestrate builds, or interact with the cache.
It produces plan nodes.
Kiln does the rest.

The adapter contract requires the adapter to answer two questions: what are my inputs, and what is the build command?
The kiln substrate handles storage, caching, verification, and distribution.
The adapter implements:

```text
<language> kiln plan → emits action nodes with inputs and dependencies
```

The adapter calls into the language's own tooling for introspection.
`cargo-kiln` uses `cargo metadata` and cargo's build plan output (`cargo build --build-plan`) to derive the dependency graph and exact `rustc` invocations; kiln then runs those `rustc` commands directly rather than invoking cargo for execution.
`uv-kiln` reads `uv.lock`.
`go-kiln` calls `go list -deps`.
The adapter is a translation layer that serializes the language tool's own understanding of the project into kiln's plan format.

This means cache key correctness is the language tool's responsibility.
It knows about feature flags, build profiles, platform-specific dependencies, and conditional compilation.
Kiln never guesses at language-specific semantics.
The adapter hashes the inputs it knows matter.
This resolves the fundamental tension in build caching: the less a build system knows about a language's internals, the coarser the caching.
The more it knows, the more it becomes language-specific.
By delegating to adapters, kiln gets fine-grained caching without centralizing language knowledge.

### Dependency Fetch Actions

The adapter emits per-crate (or per-package) fetch actions alongside build actions.
For Rust, `cargo-kiln` reads `Cargo.lock` and emits one fetch node per dependency with expected output hashes derived from the lock file's per-crate content hashes:

```toml
[[action]]
name = "vendor-serde"
isolation = 0
inputs = ["Cargo.lock"]
run = ["cargo-kiln-fetch", "--crate=serde", "--version=1.0.210"]

[outputs]
src = "vendor/serde/"

[expects]
src = "sha256:ab34ef..."
```

Build actions depend on specific vendor crate nodes:

```toml
[[action]]
name = "mylib"
isolation = 3
inputs = ["crates/mylib/src/"]

[deps.vendor-serde]
src = "vendor/serde/"

[deps.vendor-tokio]
src = "vendor/tokio/"

run = [
  "rustc", "--edition=2021", "--crate-type=lib", "--crate-name=mylib",
  "crates/mylib/src/lib.rs",
  "--extern", "serde=vendor/serde/libserde.rlib",
  "--extern", "tokio=vendor/tokio/libtokio.rlib",
  "--out-dir", "out/",
]

[outputs]
lib = "out/libmylib.rlib"
```

The `rustc` invocations are derived from cargo's build plan (`cargo build --build-plan`) rather than invoking cargo for execution.
At isolation level three, the network is blocked anyway — inputs are fully declared in the plan.

The adapter doesn't fetch anything.
It declares nodes.
Kiln executes the vendor actions (or retrieves cache hits), verifies each output against expected hashes, and stages the vendored crates as inputs to downstream build actions.
The same pattern applies to every language with a lock file: `uv-kiln` emits per-package fetch nodes reading `uv.lock`, `go-kiln` emits per-module fetch nodes reading `go.sum`.

### Planner Detection and Resolution

Planners are resolved from the environment, not discovered from PATH.
The `env.toml` file pins the planner binary as a content-addressed tree in the package store.
This avoids the ambient dependency problem — "whatever `cargo-kiln` is on PATH" is exactly the kind of undeclared input the system is designed to eliminate.

Detection is by convention.
Kiln scans the worktree for language marker files — `Cargo.toml`, `pyproject.toml`, `go.mod`, `package.json` — and invokes the corresponding planner from the environment tree.
For a project with both `Cargo.toml` and `pyproject.toml`, kiln invokes both `cargo-kiln` and `uv-kiln`, each emitting its own subgraph.

The planner version is part of the cache key for the planning action.
A planner upgrade that emits different node granularity invalidates the plan cache.
This is correct — the plan is a function of the worktree and the planner binary.

### Planner Isolation

The planning action needs an isolation level, but the planner hasn't run yet — it can't declare its isolation level in a plan node.
Instead, isolation is declared in the planner's package metadata:

```toml
# in the cargo-kiln package tree metadata
isolation = 0
```

The planner author decides the minimum. `env.toml` can raise it:

```toml
[planners]
cargo-kiln = { ref = "git://kiln-packages/cargo-kiln@0.3.0", isolation = 1 }
```

Kiln enforces the higher of the two.
Same rule as action nodes: kiln can enforce higher than declared but never lower.

### Granularity Tiers

Language adapters can emit nodes at different granularities.
At the workspace level, the entire project is one node.
The planner is trivial and caching is coarse.
At the crate or module level, one node per logical package uses moderate planning effort and achieves good caching.
At the compilation unit level, one node per compiler invocation achieves maximum caching and is the target granularity for Rust via cargo's build plan output.

For Rust, the compilation unit level is the primary target: `cargo-kiln` uses cargo's build plan to extract per-crate `rustc` invocations, which become the action `run` fields.
Kiln orchestrates rustc directly — cargo is used for planning, not execution.
For languages without a build plan equivalent (Python, Go), the module level is the practical sweet spot and the language runtime handles fine-grained details inside the sandbox.

### Codegen and Planner Limitations

Planners infer project structure from language tooling introspection.
This works well for standard dependency graphs but breaks down for codegen.
`cargo metadata` reports that a crate has a `build.rs`, but build scripts are opaque — Cargo doesn't know what files they read or generate until runtime.
`cargo:rerun-if-changed=` directives are emitted via stdout during execution, not available for static introspection.
Python has no equivalent concept at all.

This is the primary use case for `kiln.toml`.
When the planner can't infer a dependency edge — protobuf codegen, FFI bindings, generated parsers — the developer declares it explicitly.
`kiln.toml` is a sparse BUILD file: you only write the parts the planner couldn't figure out.
Users coming from Bazel will recognize this immediately — it's the same explicit declaration, but only for the edges that can't be inferred.

A sufficiently motivated planner could handle common codegen patterns.
`cargo-kiln` could detect `prost` or `tonic` in dependencies, scan for `.proto` files, and emit codegen nodes.
But this is an optimization, not a requirement.
The escape hatch always works.

### Version Metadata

Adapters also emit version metadata alongside the build graph.
Because the adapter already introspects the project, it knows where versions live.
`cargo-kiln` knows that `Cargo.toml` has a version field and understands workspace inheritance.
`uv-kiln` knows `pyproject.toml`.
This metadata enables release automation by upstream tools without separate configuration:

```toml
version:
  file: "crates/core/Cargo.toml"
  field: "version"
  current: "1.2.0"
  updater: "cargo set-version {version} -p core"
```

Adding a new crate to a Cargo workspace automatically makes it discoverable because the planner finds it.
No configuration to update.

For projects without a planner, upstream tools can fall back to scanning for known files — `Cargo.toml`, `pyproject.toml`, `package.json`, `go.mod` — and apply default patterns.

### LSP as a Future Planning Source

Language servers already maintain real-time dependency graphs for type checking and autocompletion.
An LSP knows which files import which modules, which symbols come from where, and which packages are used.
This data could supplement or replace language-specific introspection tools for planning.

However, LSP dependency graphs serve type checking, not compilation.
They don't capture build-time dependencies like build scripts, proc macros, or link-time inputs.
LSP startup is slow — rust-analyzer takes 30 to 60 seconds on a large workspace — making it unsuitable for CI where no editor is running.
And expecting LSP maintainers to implement custom extensions for a new build tool is unrealistic in the short term.

The practical approach is to use language-specific introspection tools as the primary planning path and let LSP integration evolve as an optimization.
The daemon can subscribe to LSP updates for instant plan invalidation when the developer has an editor open, making the LSP a real-time notification source rather than a query target.

### Cross-Language Composition

Each language adapter emits its own subgraph.
Cross-language edges are declared in `kiln.toml` files:

```toml
# python/service/kiln.toml
[deps.native-ext]
lib = "myservice/native.so"
```

Kiln reads `kiln.toml` files after all planners run, fetches each project's plan from the index, and stitches the DAGs together at these edges.
Within a language, the planner handles all internal dependencies.
The developer only specifies what no single language tool can know: the relationships between languages.

Every node in every graph has the same shape — input tree, action, output tree with named keys.
A node in one graph can reference named outputs of a node in another graph.
The Python side doesn't care that the native extension was built with Cargo.
It references `native-ext`'s `lib` key and gets a `.so` file at the path it declared.
The Rust side doesn't know Python will consume it.
The output keys are the interface contract — the producing node declares what's public, the consuming node declares where it goes.

Per-key dependency hashing makes cross-language composition cache-efficient.
The Python node's action hash includes only the hash of the `.so` blob it references, not the full Rust output tree.
A Rust rebuild that changes `.rmeta` metadata but produces a bit-identical `.so` does not invalidate the Python node's cache.

For external dependencies, each project publishes its output under a kiln ref.
Depending on an external project is referencing a tree hash from another Git remote:

```toml
serde = "git://github.com/serde-rs/serde@refs/kiln/manifest"
```

The consumer can trust the pre-built output or fetch the source and rebuild for verification.


## Configuration

### `env.toml`: The Environment

`env.toml` defines the content-addressed environment for the project — what toolchains are available and what planners generate the build graph.
It replaces language-native version management files (`rust-toolchain.toml`, `.python-version`, `.nvmrc`) with a single source of truth.

```toml
[toolchains]
rust = "git://kiln-packages/rust@1.82.0"
python = "git://kiln-packages/cpython@3.12.0"
postgres-client = "git://kiln-packages/postgres@16"

[planners]
cargo-kiln = "git://kiln-packages/cargo-kiln@0.3.0"
uv-kiln = "git://kiln-packages/uv-kiln@0.1.0"
```

`env.toml` is the single file that governs `kiln enter env`.
The environment assembles trees for all listed toolchains and planners, overlays them onto the worktree, and gives the developer a shell with the right tools.
The planner runs inside this environment — it doesn't resolve versions, it just calls `cargo metadata` or `uv lock` with whatever toolchain is provided.

Toolchains and planners are in the same file because the relationship between them is the most important thing to understand about the build configuration.
`cargo-kiln@0.3.0` next to `rust@1.82.0` — you see immediately what's planning what and whether versions are compatible.
Adding a language is one logical change (toolchain + planner) that should be one commit to one file.

Architecture is encoded in the ref path.
`refs/kiln/cpython/3.12.0/x86_64-linux` and `refs/kiln/cpython/3.12.0/aarch64-darwin` point to different trees.
The tool detects the local platform and fetches the matching ref.
Cross-platform environments are explicit.

For projects that need reproducible environments without the build system, `env.toml` with only `[toolchains]` provides immediate value.
The moment they adopt kiln planners, they add `[planners]` and everything is already in the right place.

### `kiln.toml`: The Escape Hatch

`kiln.toml` is optional.
It declares actions and edges that planners can't infer.
The root-level `kiln.toml` can live alongside `env.toml` or at the repository root.
Subdirectory `kiln.toml` files live alongside source at the subtree boundary — `python/service/kiln.toml`.
The root file is project-wide configuration.
Subdirectory files are owned by the team that owns that subtree.

```toml
# root kiln.toml
[[action]]
name = "protogen"
toolchain = "git://kiln-packages/protoc@27.0"
inputs = ["proto/"]
run = ["protoc", "--rust_out=gen/", "proto/api.proto"]

[action.outputs]
rust_mod = "gen/"

[deps.foo]
protogen.rust_mod = "src/generated/"
```

Kiln reads the root `kiln.toml` first, then walks the worktree for subdirectory `kiln.toml` files.
Each file's actions and edges are scoped to its subtree's nodes.
Node names in `[deps]` reference language-native package names — `foo` is the crate named `foo`, not a planner-internal identifier.
Dep entries map producing node output keys to paths in the consuming node's input tree.

`kiln.toml` is a sparse BUILD file.
The spectrum from "planner does everything" to "fully manual" is how much you write in `kiln.toml`.
If the overrides replace everything a planner emitted, that's effectively a new plan.
No special case needed — it's the escape hatch turned to full.


## The Daemon

### Purpose

The daemon is an optional local process that keeps the object store hot and watches for changes.
If it is running, builds are faster.
If it is not, everything works the same, just cold.

The daemon watches the worktree.
On every file save, it re-hashes affected subtrees, recomputes action hashes, and checks the cache.
By the time the developer runs a build command, the daemon already knows which nodes are cache hits and which need building.
Planning is done and the build starts executing immediately.

The daemon also watches `env.toml`.
When toolchain or planner refs change, it fetches the new trees in the background.
By the time the developer runs `kiln build` after updating a toolchain, the trees are already local.

### Speculative Builds

The daemon can observe editing patterns and start building proactively.
When it sees changes to `crates/core/src/lib.rs`, it knows core's dependents will need rebuilding.
It can start building core as soon as the file is saved, before the developer asks.
By the time a full build is requested, core is already done.

### Background Prefetch

When the developer pulls, the daemon sees new commits.
It fetches signed output refs from the remote for those commits' worktree hashes in the background.
By the time a build runs, the cache is already populated locally.

### Plan Caching Across Branches

Switching branches produces a different worktree hash.
The daemon diffs the two trees, identifies which nodes are affected, and only re-plans those.
The rest of the DAG carries over from the previous branch's plan.


## Signed Builds and Verification

### The Trust Model

Only CI runners can sign commits on the build graph.
Local builds populate the local cache.
The shared remote only accepts build refs signed by CI keys.
Consumers verify the signature before trusting a cached output.

The developer pushes source code.
CI pulls, builds at whatever isolation level the project requires, and signs the output refs.
Anyone can verify that the output was built by CI from the declared inputs.
Anyone can also rebuild from source and check that their output matches CI's.

Unsigned local cache is a convenience for the developer's own machine.
Signed CI cache is the source of truth for everyone else.
The signature attests not just that CI built the artifact but that CI built it at a specific isolation level from a specific input tree.

### Build Graph as Git History

Each target has its own ref at `refs/kiln/outputs/<name>/<profile>/<arch>`.
Each build of that target produces a signed commit on the ref.
The commit's tree is the build output.
The commit's first parent is the previous build of that same target (version history).
Additional parents are the output commits of the target's dependencies (provenance DAG).

For example, building `mylib` which depends on `serde`:

```text
refs/kiln/outputs/vendor-serde/release/x86_64-linux → commit VS1 (signed)
  tree: vendor/serde/
  parents: (none, first build)
  message: action=vendor-serde, source=<lock_hash>, isolation=0, verified=sha256:ab34ef...

refs/kiln/outputs/serde/release/x86_64-linux → commit S1 (signed)
  tree: deps/libserde.rlib, deps/libserde.rmeta, .fingerprint/...
  parents: VS1 (vendor-serde output)
  message: action=serde, source=<tree_hash>, toolchain=<rust_hash>, isolation=3

refs/kiln/outputs/mylib/release/x86_64-linux → commit M1 (signed)
  tree: deps/libmylib.rlib, deps/libmylib.rmeta, .fingerprint/...
  parents: M0 (previous mylib build), S1 (serde output)
  message: action=mylib, source=<tree_hash>, toolchain=<rust_hash>, isolation=3
```

`git log M1` walks the full DAG: M1 ← S1 ← VS1. `git verify-commit` on any node proves CI built it. `git diff S1_old S1_new` shows what changed when serde was bumped. `git log refs/kiln/outputs/mylib/release/x86_64-linux` shows every build of mylib over time.

The final deliverable for a release is a merge commit whose parents are the individual build commits.
The resulting tree is the complete build output.
Provenance is preserved through the commit structure.
The source repository has its history.
The build graph has its own parallel history.
They are linked by the input hashes in commit messages.

The cache index (`refs/kiln/cache`) is a separate metadata ref that maps action hashes to commit OIDs for fast lookup.
It is derived, not authoritative — if lost, it can be rebuilt by walking the target refs.
See the Build DAG and Cache Index section.

### Architecture Encoding

Architecture and OS are part of the cache key.
The plan's target field is part of the action hash.
Same source compiled for different targets produces different cache keys.
There are no accidental cross-architecture cache hits.

CI builds per target in a matrix.
Each target produces a separate set of signed output refs.
A consumer machine detects its local target, computes action hashes with that target, and looks for signed output matching those hashes.
If CI didn't build for that target, there is no cache entry and the consumer builds locally.
The toolchain tree is also part of the cache key — a build against one base environment versus another produces a different action hash.

### Nondeterminism

Some compilers do not produce bit-identical output across runs.
When verification fails — the local rebuild produces a different hash than CI's signed output — the outputs can be diffed with Git to trace the difference.
Depending on the index structure, a bisect can help locate the divergence.

At isolation levels below two, the system defaults to trusting the last binary produced rather than demanding hash equality.
Strict verification applies only at higher isolation levels where the build environment is controlled enough to expect determinism.
This is an honest tradeoff: verification works for the common case, and the cases where it fails are diagnosable rather than silent.


## Environments

### Git Trees as Environments

The `env.toml` file defines the project environment as content-addressed Git trees.
`kiln enter env` assembles the trees for all listed toolchains and planners, overlays them onto the worktree, and gives the developer a shell with the right tools.
This is `kiln enter` applied to an environment action — the environment is just an action whose output is a usable toolchain tree.

`env.toml` replaces language-native version management.
Instead of `rust-toolchain.toml` telling rustup which tarball to download, `env.toml` pins a content-addressed tree.
Instead of `.python-version` telling pyenv which Python to activate, `env.toml` pins a content-addressed tree.
The role is the same — "when you enter this project, use this toolchain" — the mechanism is different.

For Rust, this is a clean substitution.
`cargo` doesn't read `rust-toolchain.toml` — rustup does.
Cargo just calls whatever `rustc` is on PATH.
Kiln provides the `rustc`, no rustup needed.

For Python, `uv` reads `.python-version` and uses the Python version as a dependency resolution input via `requires-python` in `pyproject.toml`.
But these serve different purposes: `requires-python` is a constraint (minimum bound), `env.toml` is the concrete pin.
They don't conflict.
If `env.toml` changes Python from 3.12 to 3.13, `uv.lock` may need regeneration — but that's correct behavior, because changing a toolchain should invalidate dependency resolution.

### Bootstrapping Packages

Initial toolchain packages are bootstrapped from official release binaries imported into the Git object store.
A `kiln import` command fetches from official release URLs and writes trees locally.
The binaries never pass through a kiln-hosted server, avoiding redistribution concerns.

As the build system matures, bootstrap packages are replaced with kiln-built ones.
The same ref gets a new tree hash backed by a signed build graph.
Consumers don't change anything.
Trust properties improve transparently.

For a healthy ecosystem, community-maintained package repositories follow the Arch Linux model: comply with licensing per package, make source available for copyleft-licensed tools, and exclude proprietary software from public repos.
Proprietary toolchains live in private stores.

### Path to Replacing Docker

The progression moves through clear stages.
Initially, Kiln builds artifacts and Docker assembles images from those artifacts.
Next, Kiln assembles the root filesystem tree and Docker just runs it via `docker import`.
Eventually, Kiln assembles and runs via `unshare` and `chroot` with no Docker involvement.

At every stage, export to OCI format provides compatibility with the existing container ecosystem.


## Remote Build Execution

### Architecture

Kiln already has the primitives for remote execution.
An action is a self-contained unit of work: input tree hash, action blob, expected output tree hash.
It can be sent anywhere.

On a cache miss that isn't satisfied locally or from the remote cache, kiln submits the action to an RBE worker pool.
A worker fetches the input tree from the Git remote, materializes it into a sandbox, executes the action, and pushes the output tree back to the remote.
Trusted workers sign the output.

Workers are dumb.
They receive an action hash and an input tree hash.
They have no knowledge of the project, the language, or the DAG.
They fetch blobs, run a command in a container, and push the result.
Workers are stateless and share nothing except the Git remote.

### Simplicity Over Bazel RBE

Bazel's Remote Execution API defines a complex protobuf protocol with dedicated servers.
Kiln's version uses Git's transport protocol directly.
The worker is a Git client.
It fetches with `git fetch` and pushes with `git push`.
No custom API, no dedicated build farm servers.

Scaling is straightforward: add more workers.
Two workers building the same action is harmless — both produce the same output hash, last push wins.


## Licensing

### Cache as Distribution

Build cache entries stored on a Git remote are accessible to anyone who can fetch from that remote.
For private repositories, cache access is scoped to the organization — sharing compiled artifacts internally is not "distribution" under copyright law.
This is the same legal posture as a company's existing CI cache, Artifactory, or S3 artifact bucket.

For public repositories, build refs under `refs/kiln/outputs/` are fetchable by anyone, even though the default refspec doesn't include them.
This constitutes distribution.
The licensing implications depend on what the cache contains and what licenses govern the dependencies.

### Compiled Artifacts and Dependency Licenses

Most open-source licenses (MIT, Apache-2.0, BSD) permit redistribution of compiled derivatives with attribution.
A `NOTICE` file in the output tree satisfies this.
The planner already has license metadata from dependency manifests — generating the notice file as part of the build output is trivially automated.

Copyleft licenses (GPL, LGPL, AGPL) require that recipients of compiled artifacts can obtain the complete corresponding source.
For a public open-source project, the source is in the same public repository.
The vendored source tree (from the vendor actions) provides an even stronger guarantee — the exact source used to produce the binary is in the same Git object store, fetchable by hash.

### Vendored Source in the Cache

The vendor actions' output trees contain copies of each dependency's source.
Redistributing these trees is redistributing those dependencies' source code.
For permissive licenses, this is fine with attribution.
For copyleft licenses, this is actually desirable — GPL requires source availability, and the vendor trees provide it.

The only problematic case is proprietary dependencies with redistribution restrictions.
These should not appear in a public cache.
A license enforcement policy can prevent this by denying builds that include disallowed licenses.


## CLI

```text
kiln
├── build [target] [--profile=P] [--isolation=N]
├── plan [target]
├── materialize <action> [--output=key]
├── enter <action> [--isolation=N]
├── import <url>
├── export --oci <action>
├── cache
│   ├── status
│   ├── prune
│   └── fetch
└── status
```

All list commands support `--json` for scripting.
Every subcommand maps to a ref operation.
There is no hidden state outside Git.


## Development Roadmap

### Phase 1: Single-Node Actions

Implement the kiln executor with single-node plans and Docker-based execution.
A project's entire build is one action — the worktree is the input, Docker provides the toolchain, the build script is the command.
This is the universal plan that works for every language with zero configuration.
No planners, no language awareness, no adapters.
But the caching model, the ref format, the signed output infrastructure, and action failure semantics are real from day one.

### Phase 2: Isolation Level One

Implement read-only input enforcement.
Builds run on the host but the input tree is mounted read-only and writes go to a capture directory.
This works on both Linux and macOS (with weaker enforcement on macOS via filesystem permissions).
This is the minimum isolation needed for planner development — without at least this level, a planner can emit incomplete inputs and nobody notices.

### Phase 3: First Planner

Build the first language adapter, either `cargo-kiln` or `uv-kiln`.
The planner calls into the language's own introspection tools and emits multi-node plans at the crate or package level with profile-agnostic DAGs and per-node profile overrides.
The planner also emits per-crate vendor fetch actions with expected output hashes derived from the lock file.
This phase validates whether language tooling exposes enough information for correct cache keys and whether the node format is sufficient without language-specific extensions.

### Phase 4: Output Modes

Implement `kiln materialize` and `kiln enter`.
This phase validates the three output modes (default state materialization, explicit artifact materialization, transient context) and forces solutions to tree composition and platform-specific mounting mechanisms.
On macOS, `kiln enter` at level zero uses tmpdir copy or symlink farms.

### Phase 5: Isolation Level Two

Implement filesystem isolation via mount namespaces on Linux.
Only declared inputs are visible to the build.
On macOS, this level delegates to a Linux VM.
This is where builds become genuinely reproducible and where the planner's input declarations are strictly enforced.

### Phase 6: Second Planner

Build the second language adapter targeting a language with a very different build model from the first.
If the plan format needs language-specific fields to accommodate the second language, the abstraction is leaking.
If it works cleanly, the spec is sound.

### Phase 7: Cross-Language Composition

Wire `kiln.toml` dependency edges between projects using different languages.
A project with a Rust native extension consumed by a Python service, built end-to-end by `kiln build`, is the reference test case.

### Phase 8: Isolation Level Three

Add network isolation.
Verified fetch actions enable projects to acquire external dependencies while maintaining a level-three policy.
Combined with CI signing, this level enables full build verification.

### Phase 9: Environments

Implement `kiln enter env` with bootstrapped toolchain packages imported from official release binaries via `kiln import`.
Developers get reproducible environments as Git trees defined by `env.toml`.

### Phase 10: Remote Build Execution

Implement remote workers that fetch input trees, execute actions in sandboxes, and push signed output trees.
Workers are stateless Git clients.

### Deferred Indefinitely

Isolation level four — fixed timestamps, PID namespaces, deterministic file ordering — provides diminishing returns for enormous effort.
Deferred until clear demand materializes.
