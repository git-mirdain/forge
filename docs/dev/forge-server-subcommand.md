# `forge server` subcommand

## Problem

`forge-server` is a standalone binary that syncs forge refs with GitHub.
Users must discover, install, and run it separately from the `forge` CLI.
Making it a subcommand gives single-binary distribution and discoverability.

## Approach

### 1. Extract forge-server logic into a library

Add `lib.rs` to the `forge-server` crate exposing the core sync loop.
Keep `main.rs` as a thin wrapper so the standalone binary still works.

- `forge-server/src/lib.rs` — exports `run()`, `SYNC_REF_PREFIXES`,
  `sync_one()`, `fetch_forge_refs()`, `push_forge_refs()`, and
  `ServerConfig` (a plain struct replacing the clap `Args`)
- `forge-server/src/main.rs` — parses clap args, builds `ServerConfig`,
  calls `forge_server::run()`

### 2. Add `server` feature to git-forge

The `server` feature gates the `forge server` subcommand.
No new crate dependencies — `git-forge` cannot depend on `forge-server` or `forge-github` because both already depend on `git-forge` (cyclic).

Instead, `forge server start` spawns `forge-server` as a detached child process and manages it via a pidfile at `.git/forge-server.pid`.

```toml
server = ["cli"]
default = ["cli", "exe", "server"]
```

### 3. Add `ServerCommand` to CLI

Behind `#[cfg(feature = "server")]`:

```rust
enum ServerCommand {
    Start {
        poll_interval: u64,   // default 60
        remote: String,       // default "origin"
        no_sync_refs: bool,
        once: bool,
        foreground: bool,
    },
    Stop,
    Status,
}
```

### 4. Implement in executor

Behind `#[cfg(feature = "server")]`:

- **start**: prompt "Start forge sync daemon?" (unless `--foreground` or
  `--once`), spawn `forge-server` as a detached subprocess, write pid to
  `.git/forge-server.pid`
- **stop**: read pidfile, send SIGTERM via `kill`, remove pidfile
- **status**: read pidfile, check if process alive via `kill -0`

### Files touched

| File | Change |
|------|--------|
| `crates/forge-server/src/lib.rs` | **New** — extract sync logic |
| `crates/forge-server/src/main.rs` | Thin wrapper calling lib |
| `crates/git-forge/Cargo.toml` | Add `server` feature |
| `crates/git-forge/src/cli.rs` | Add `ServerCommand` |
| `crates/git-forge/src/exe.rs` | Add server dispatch |
