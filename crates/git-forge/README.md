# ⚒️ `git-forge`

*CLI entrypoint and library facade for local-first Git forge infrastructure.*

<!-- rumdl-disable MD013 -->
[![CI](https://github.com/git-ents/git-forge/actions/workflows/CI.yml/badge.svg)](https://github.com/git-ents/git-forge/actions/workflows/CI.yml)
[![CD](https://github.com/git-ents/git-forge/actions/workflows/CD.yml/badge.svg)](https://github.com/git-ents/git-forge/actions/workflows/CD.yml)
<!-- rumdl-enable MD013 -->

> [!CAUTION]
> This project is in active development.
> There are surely bugs and misbehaviors that have not yet been discovered.
> Please file a [new issue] for any misbehaviors you find!

[new issue]: https://github.com/git-ents/git-forge/issues/new

## Overview

This crate is the top-level entry point for the `git-forge` workspace.
It wires together the domain crates — issues, reviews, and releases — into a single `git forge` CLI and re-exports them as a unified library facade.

## Installation

### CLI

The `git-forge` command can be installed with `cargo install`.

```shell
cargo install --locked git-forge
```

If `~/.cargo/bin` is on your `PATH`, you can invoke the command with `git`.

```shell
git forge -h
```

### Library

The `git-forge` library can be added to your Rust project via `cargo add`.

```shell
cargo add git-forge
```
