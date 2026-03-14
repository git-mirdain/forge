# 🚀 `git-forge-release`

*Release management for local-first Git forge infrastructure.*

> [!CAUTION]
> This project is in active development.
> There are surely bugs and misbehaviors that have not yet been discovered.
> Please file a [new issue] for any misbehaviors you find!

[new issue]: https://github.com/git-ents/git-forge/issues/new

## Overview

This crate implements the release management domain for `git-forge`.
Releases are stored as Git refs under `refs/meta/releases/`, with each mutation recorded as a new commit, giving every release a built-in audit log.

## Installation

The `git-forge-release` library can be added to your Rust project via `cargo add`.

```shell
cargo add git-forge-release
```
