+++
title = "Hearth: Environments as Git Trees"
subtitle = "Design Specification"
version = "0.2.0"
date = 2026-03-23
status = "Draft"
summary = """
Hearth is an environment manager backed by Git's content-addressed object store.
It treats build environments as pure compositions of content-addressed filesystem
trees, providing reproducible, inspectable, and shareable environments with
graduated isolation — from convention-only to full VM-based sandboxing."""
+++

# Hearth: Environments as Git Trees

## Foundation

### The Problem with Existing Environment Management

Environment management today is split across tools that don't compose: `rustup` manages Rust toolchains, `pyenv` manages Python versions, `nvm` manages Node, Docker manages containers.
Each tool has its own storage, its own update mechanism, its own notion of "current version."
None of them are content-addressed.
None of them are reproducible in the strong sense.
None of them share artifacts across projects.

The deeper problem is that these tools manage *versions*, not *content*.
`rust@1.82.0` is a name.
The actual bytes of the compiler are not tracked.
Two machines with `rust@1.82.0` installed may have different bytes if one installation was corrupted, patched, or modified.
The version string provides no guarantee about content.

Hearth replaces version management with content management.
A Hearth environment is a named composition of content-addressed filesystem trees.
The environment hash is derived from the content of every file in every component tree.
Two machines with the same environment hash have identical environments — provably, by construction.

### Hearth Is Not a Package Manager

Hearth does not distribute toolchains.
It does not host packages.
It does not resolve version constraints.
It does not know what `rust@1.82.0` is.

Hearth takes trees you already have — imported from official release tarballs, pulled from OCI registries, built by Kiln, copied from a colleague — and composes them into environments identified by a content hash.
Where the trees came from is not Hearth's concern.
What they contain is fully tracked.

This is a deliberate scope constraint.
Package distribution is a solved problem with many existing solutions.
Environment composition and activation is not.

### Environments as Git Trees

A Hearth environment is a Git tree object.
Every file in the environment is a Git blob.
The environment hash is the root tree hash.
This is not a metaphor — Hearth uses Git's object database directly, via libgit2, as its storage layer.

This means:

- Environments are inspectable with standard Git tools: `git cat-file -p <hash>`
- Environment diffs are `git diff <hash-a> <hash-b>`
- Environment transport is `git fetch` and `git push`
- Deduplication is structural: files shared across environments are stored once
- Signing is `git verify-commit` on build output commits

The environment is not an opaque artifact.
It is a tree of blobs that any sufficiently motivated person can inspect, reproduce, or reconstruct manually.
This is the meaningful guarantee of user ownership — not that no VM is involved, but that nothing is hidden.

### Relationship to Kiln

Hearth and Kiln are separate tools with a clean interface between them.

Hearth's responsibility: tree composition and environment activation.
It assembles component trees, manages the content-addressed store, and presents environments to processes via the appropriate platform mechanism.

Kiln's responsibility: build execution and caching.
It consumes environments provided by Hearth, executes actions inside them, and stores build outputs as Git trees.

Hearth does not execute builds.
Kiln does not manage toolchain trees.
The interface between them is a Git tree hash — Hearth produces it, Kiln consumes it.
Both share the same Git object store, so there is no duplication of blobs across the two tools.

## Architecture

### The Store

Hearth maintains a content-addressed store in two layers:

**Git object store** — the source of truth.
Every blob, tree, and environment ref lives here.
Backed by libgit2.
Deduplication is free: writing a blob that already exists is a no-op.

**Materialized store** — a local disk cache of checked-out trees for platform consumption.
Lives at `~/.hearth/store/`.
Not tracked by Git.
Reconstructable at any time from the Git object store.
Structured as a flat blob cache with hardlinks:

```text
~/.hearth/
  objects/              ← Git object store (libgit2)
  blobs/<hash>          ← one checked-out copy of each blob on disk
  store/<tree-hash>/    ← hardlinks into blobs/, structured as a filesystem tree
  runs/<id>/            ← per-invocation capture directories
```

The blob cache is the single on-disk copy of each file's content.
Store entries are hardlinks into it.
Two environments sharing a file share one inode.
The hardlink count on a blob cache entry is its reference count — when it drops to 1 (only the cache entry itself), the blob is eligible for `hearth gc`.

### Ref Namespace

```text
refs/hearth/trees/<hash>        → imported component tree
refs/hearth/envs/<env-hash>     → merged environment tree
refs/hearth/kernels/<hash>      → vmlinuz blob
```

Component tree refs are what `hearth import` writes.
Environment refs are derived by merging component trees — they are an index, not authoritative.
If lost, they can be reconstructed by remerging.
The environment hash is the merged tree hash, computable without materializing anything.

### Component Trees

A component tree is any Git tree object in Hearth's object store.
It represents a filesystem subtree — a toolchain, a set of libraries, a configuration tree, a build output promoted from Kiln.
Hearth does not distinguish between these origins.
A tree is a tree.

Trees are imported via `hearth import` from whatever source the user controls: a release tarball, an OCI image, a local directory, a Kiln build output.
After import, the tree is identified by its hash.
The import source is not tracked — the content hash is the identity.

### Environment Composition

An environment is declared in `env.toml` using tree hashes directly:

```toml
[env.default]
trees = [
  "a3f1c9d...",   # rust toolchain, imported by user
  "b72e4f8...",   # python, imported by user
]

[env.dev]
extends = "default"
trees = ["c91a3b2..."]   # postgres client libs

[env.ci]
extends = "default"
isolation = 3
```

Tree hashes are the only reference.
There are no version strings, no package names, no remote URLs resolved at activation time.
The environment is fully specified by its component hashes.

Hearth computes the merged environment tree by:

1. Fetching each component tree object from the local store
2. Merging trees in declaration order — last-wins on conflict, deterministic
3. Writing the merged tree as a new Git object
4. Storing the ref: `refs/hearth/envs/<merged-tree-hash>`

The merged tree hash is the environment hash.
It is computable from component tree hashes alone, without materializing any files.

**Conflict resolution:** When two component trees provide the same path, the later entry in the `trees` list wins.
Order is declaration order.
This is documented behavior, not an error.

## Isolation Levels

Hermeticity is a dial.
Hearth defines four isolation levels, reflecting honest platform capabilities.

### Level 0 — Convention Only

The process runs on the host.
Hearth manipulates `PATH` and environment variables to point at the materialized toolchain tree.
No filesystem enforcement.
The process can read and write anything the user can.

Useful for: development workflows where speed matters more than strict isolation.

**Available on:** Linux, macOS, Windows.

### Level 1 — Read-Only Inputs

The materialized store is chmod'd read-only (`a-w`) before the process starts.
Writes are redirected to a per-invocation capture directory (`~/.hearth/runs/<id>/capture/`).
The process can still read arbitrary host files and access the network, but it cannot mutate component trees.

On macOS, the store is materialized via hardlinks from the blob cache.
Read-only enforcement via chmod propagates correctly to hardlinked files because it operates on the inode.

Useful for: catching accidental mutations of toolchain inputs.

**Available on:** Linux, macOS (permission bits only, no mount isolation), Windows (NTFS ACLs, similar caveats).

### Level 2 — Filesystem Isolation

The process runs in a mount namespace (Linux) or Linux VM (macOS, Windows).
Only declared trees are visible.
The host filesystem is not accessible.
Network access is permitted.

On Linux: `unshare -m` + bind mounts.
The toolchain tree is bind-mounted read-only.
A tmpfs is mounted for writes.
No container runtime required.

On macOS and Windows: a Linux VM via Virtualization.framework (macOS) or Hyper-V/WSL2 (Windows).
The toolchain tree is shared into the VM via virtio-fs.
The VM root filesystem is itself a Git tree materialized on the host and shared read-only.
No opaque disk image.
Every file in the VM is a Git blob.

**Available on:** Linux natively. macOS and Windows via Linux VM.

### Level 3 — Network Isolation

Level 2 plus a network namespace.
No network access.
All dependencies must be in the declared input trees.

On Linux: `unshare -mn`.

On macOS and Windows: network isolation configured inside the VM.

**Available on:** Linux natively. macOS and Windows via Linux VM.

### A Note on macOS and Windows

Strong isolation (levels 2+) on macOS and Windows requires a Linux VM.
This is not a limitation to paper over — it is a consequence of platform capability. macOS exposes no mount namespace equivalent available to userspace.
`sandbox-exec`, used by Nix and Bazel, is deprecated, weaker, and not maintained by Apple.
Hearth does not use it.
There is no userspace path to filesystem isolation on macOS without a kernel extension, and kernel extensions are being phased out by Apple with no equivalent replacement for this use case.

Apple's answer is Virtualization.framework — a first-class framework shipping since macOS 12, no kernel extension required, starting in approximately one second when warm.
That is what Hearth uses.

Users whose workflows only require levels 0–1 pay no VM cost.
Users who require hermetic builds accept the VM as necessary infrastructure.

## The VM Root Filesystem

The Linux VM does not use a disk image.
Its root filesystem is a Git tree, materialized on the host and shared into the VM via virtio-fs.
This means:

- The VM filesystem is content-addressed and reproducible
- Every file is a Git blob, inspectable with standard tools
- The VM root is imported and stored like any other component tree
- `hearth diff <env-a> <env-b>` can show VM root differences

The minimal VM root tree contains:

- A minimal init (not systemd — fast startup)
- musl libc
- Basic `/proc`, `/dev`, `/sys` mount points
- A shell for interactive `hearth enter` use

The Linux kernel is stored as a Git blob under `refs/hearth/kernels/<hash>`.
It is loaded by Virtualization.framework before the filesystem mounts.
The kernel hash is declared in `env.toml` and is part of the environment hash computation.
Different environments can use different kernels.

```toml
[vm]
kernel = "d4f2a1e..."   # hash of vmlinuz blob, imported by user
root = "e83c1a9..."     # hash of VM root filesystem tree, imported by user
```

Both the kernel and root filesystem are imported by the user via `hearth import`.
Hearth does not ship or suggest either.

## Importing Trees

`hearth import` is the boundary between the outside world and Hearth's object store.
It accepts arbitrary sources and produces a tree hash.

```text
hearth import tarball <path-or-url> [--strip-prefix=N]
hearth import oci <image-ref>
hearth import dir <local-path>
```

After import, the tree is in the local object store identified by its hash.
How the user obtained the source, what they trust about it, and where they record the resulting hash are outside Hearth's scope.

**OCI import determinism:** OCI layers are unpacked in order with whiteout handling, producing a single merged filesystem tree.
The unpacking must be deterministic — timestamps are zeroed, xattrs are handled consistently, hardlinks are preserved.
The same OCI image always produces the same tree hash on every machine.
This is a correctness requirement, not a best-effort property.

**The macOS SDK:** Cross-compilation to macOS targets requires Apple's SDK, which is proprietary and cannot be redistributed under Apple's Xcode license.
Users import it from their own licensed Xcode installation:

```text
hearth import dir /Applications/Xcode.app/Contents/Developer/Platforms/\
  MacOSX.platform/Developer/SDKs/MacOSX.sdk
```

The resulting hash is a component tree usable in environments that need macOS cross-compilation.
Hearth cannot provide this tree.
The user must.

## Output Modes

### `hearth enter <env> [--isolation=N]`

Presents a transient shell inside the environment.
On exit, the host is untouched.
Writes land in the capture directory and are discarded unless explicitly promoted via `hearth import dir`.

On Linux: bind mounts + optional namespaces.
Fast, no copies.
On macOS/Windows at level 2+: VM with virtio-fs share.

### `hearth materialize <env> [--path=P]`

Writes the merged environment tree to a path on disk.
Useful for tools that need a real filesystem — direnv integration, editor toolchain configuration, CI environment setup.

```bash
# .envrc
eval "$(hearth materialize default --direnv)"
```

The materialized path is a hardlink farm from the blob cache.
No copies.

### `hearth hash <env>`

Prints the deterministic environment hash without materializing anything.
Stable across machines given the same component trees.

### `hearth diff <env-a> <env-b>`

Shows filesystem differences between two environments as a Git diff.
Works across versions, machines, and time.

## CLI

```text
hearth
├── enter <env> [--isolation=N]
├── materialize <env> [--path=P]
├── hash <env>
├── diff <env-a> <env-b>
├── import
│   ├── tarball <path-or-url> [--strip-prefix=N]
│   ├── oci <image-ref>
│   └── dir <local-path>
├── gc
└── status
```

No hidden state outside the Git object store and the materialized store.
Everything is reconstructable from the object store alone.

## User Ownership

A binary produced inside a Hearth environment is fully owned by the user:

- The exact toolchain is a content-addressed tree hash — inspectable and
  reproducible without Hearth installed
- The build commands are standard tool invocations readable from the Kiln plan
- The source is a Git commit
- The build can be reproduced manually given the tree hash and the build commands

The VM does not undermine this.
The VM root filesystem is a Git tree.
The toolchain is a Git tree.
Nothing is opaque.
A sufficiently motivated user can reconstruct the full environment from object hashes alone.

## Relationship to Nix

Nix and Hearth share the insight that content-addressed inputs produce reproducible outputs.
The differences are scope and honesty about platform constraints.

Nix is a package manager, system configuration tool, and build system.
Hearth is only an environment manager.
It makes no attempt to be the other things Nix is.

Nix on macOS uses `sandbox-exec` and produces builds the Nix community treats as less trustworthy than Linux builds.
Hearth uses a Linux VM on macOS and makes no pretense of native isolation equivalence.

Nix's store paths encode the hash in the path itself, conflating cache key with output location.
Hearth stores everything as Git objects, making standard Git tooling applicable to environments without special-casing.

## Development Roadmap

### Phase 1: Store and Import

Implement the Git object store layer via libgit2.
`hearth import tarball`, `hearth import oci`, `hearth import dir` with deterministic tree construction.
`hearth hash` and `hearth diff`.
No environment activation yet.

This phase is the correctness-critical core: blob writing, tree construction, hash stability, OCI import determinism, hardlink materialization.
These must be right before anything else is built on top.

### Phase 2: Level 0 and Level 1

`hearth enter` and `hearth materialize` at isolation levels 0 and 1 on Linux and macOS.
PATH manipulation, environment variable control, read-only enforcement via chmod.
Capture directory for writes.
`hearth gc`.

### Phase 3: Linux Level 2 and 3

Mount namespaces, bind mounts, network namespaces on Linux via `unshare`.
No container runtime dependency.
VM root filesystem tree for consistent semantics between native and VM-based execution.

### Phase 4: macOS VM

Virtualization.framework integration.
Minimal Linux VM with virtio-fs root filesystem.
`hearth enter` at levels 2 and 3.
VM warm-start target: under one second.

### Phase 5: Windows

WSL2 or Hyper-V backend.
Levels 0 and 1 natively.
Levels 2 and 3 via Linux VM.

### Phase 6: Kiln Integration

Formal integration with Kiln's action hash computation.
Environment tree hash as a component of Kiln action hashes.
Confirmed shared object store with no blob duplication.
