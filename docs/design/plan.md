# Hearth Demo Development Plan

## Context

Greenfield Rust project.
The goal is a working demo where hearth manages its own build environment: import a Rust toolchain, compose an environment from `env.toml`, and develop inside it.
Phases 1–2 of the design doc roadmap, targeting macOS, Level 0 isolation (Level 1 as stretch).

Demo story: bootstrap hearth's own development inside a hearth environment.

## Store Design: Project-Local

Hearth objects live in the project's own `.git`.
Refs are namespaced under `refs/hearth/`.
The materialization cache lives at `.hearth/` (gitignored).

```text
.git/                          ← project repo, hearth writes objects here
  refs/hearth/trees/<hash>     ← imported component trees
  refs/hearth/envs/<hash>      ← merged environment trees
.hearth/                       ← materialization cache, gitignored
  blobs/<hash>                 ← decompressed blob content (one copy per unique file)
  store/<tree-hash>/           ← hardlink farms into blobs/
  runs/<id>/                   ← per-invocation capture dirs
```

`Store` takes a `git2::Repository`.
Today it's the project repo; later it can be a separate repo or use git alternates.
Both the materialization directory (default `.hearth/`) and the ref prefix (default `refs/hearth/`) are configurable by library consumers (e.g. Kiln).

No `hearth init` — opening the store discovers the project's `.git`.

## env.toml

`[env]` is the root environment.
Named environments are `[env.<name>]`.

```toml
[env]
trees = ["abc123...", "def456..."]

[env.dev]
extends = "env"
trees = ["789abc..."]
```

Merge strategy is configurable per-environment (defaults to `last-wins`).

## Scope

**In:** `hearth import dir`, `hearth import tarball`, `hearth enter`, `hearth run`, `hearth materialize`, `hearth hash`, `hearth diff`, `hearth status`, `hearth gc`, `env.toml` parsing.

**Out:** OCI import, isolation levels 2+, VM, Windows, Kiln integration, global/shared store.

## Module Layout (`crates/hearth/src/`)

```text
src/
  main.rs            — clap CLI entry point
  lib.rs             — public API re-exports
  cli.rs             — CLI root (clap derive)
  cli/
    import.rs        — hearth import {dir,tarball}
    enter.rs         — hearth enter
    run.rs           — hearth run
    materialize.rs   — hearth materialize
    hash.rs          — hearth hash
    diff.rs          — hearth diff
    status.rs        — hearth status
    gc.rs            — hearth gc
  store.rs           — Store struct (wraps git2::Repository + materialization paths)
  store/
    blob.rs          — blob read/write via git2
    tree.rs          — tree construction, merge, walk
    refs.rs          — ref namespace management
  import.rs          — import dispatcher
  import/
    dir.rs           — import from local directory
    tarball.rs       — import from .tar.gz
  env.rs             — Environment struct
  env/
    config.rs        — env.toml parsing (serde + toml)
    compose.rs       — tree merge dispatch
    merge.rs         — MergeStrategy trait definition
    merge/
      last_wins.rs   — last-wins strategy (only impl for now)
  activate.rs        — activation dispatcher
  activate/
    level0.rs        — PATH/env var manipulation, shell spawn
  materialize.rs     — hardlink farm from blob cache
```

## Implementation Steps

### Step 1: Store foundation

- `Store` struct wrapping `git2::Repository` (discovered via `discover(".")`)
- Configurable materialization dir and ref prefix
- On open: ensure materialization dir exists (`blobs/`, `store/`, `runs/`), add to `.gitignore`
- Blob write/read, tree construction from `(path, blob_oid)` entries
- Tree merge: N tree OIDs + a `MergeStrategy` → merged tree
  - `MergeStrategy` is a trait; sole impl is `LastWins` (later entry wins on conflict)
  - Strategy declared per-environment in `env.toml`, defaults to `last-wins`
- Ref read/write under configurable prefix

### Step 2: Import (dir)

- Walk a local directory, write each file as a blob, construct the tree, store ref
- `hearth import dir <path>` → prints tree hash
- Deterministic by construction (git trees are sorted)

### Step 3: Import (tarball)

- Unpack `.tar.gz` into blobs + tree (xz/zstd deferred)
- `--strip-prefix=N` support
- `hearth import tarball <path>` → prints tree hash

### Step 4: env.toml and environment composition

- Parse `env.toml` with serde: `[env]` root, `[env.<name>]` named environments
- Resolve `extends` chains
- Compose trees via configurable merge strategy
- `hearth hash [env]` → prints merged tree hash
- `hearth diff <env-a> <env-b>` → git2 diff on two resolved trees

### Step 5: Materialize

- Walk resolved tree, write each blob to `.hearth/blobs/<sha>` (decompressed, once)
- Create `.hearth/store/<tree-hash>/` with hardlinks into `blobs/`
- `hearth materialize [env] [--path=P]`

### Step 6: Enter and Run (Level 0)

- Materialize environment if not cached
- Prepend materialized `bin/` to `PATH`, set `HEARTH_ENV`, `HEARTH_ROOT`
- `hearth enter [env]` — interactive shell
- `hearth run [env] [--] <cmd...>` — exec a command

### Step 7: Status and GC

- `hearth status` — imported trees, composed envs, disk usage
- `hearth gc` — remove blobs with hardlink count == 1, prune empty store dirs

### Step 8: Polish

- Error messages, `--help` text
- Demo workflow: download Rust tarball → import → write `env.toml` → `hearth enter` → `cargo build`

## New Dependencies

- `toml`, `serde` — env.toml parsing
- `flat2`, `tar` — tarball import
- `walkdir` — directory traversal

## Verification

```bash
hearth import dir ~/.rustup/toolchains/stable-aarch64-apple-darwin/
# → abc123...
# write env.toml with that hash under [env]
hearth hash
hearth materialize
hearth enter            # interactive shell
hearth run -- rustc --version
hearth run -- cargo build
```
