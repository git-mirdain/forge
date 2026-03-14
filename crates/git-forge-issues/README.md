# 📋 `git-forge-issues`

*Issue tracking for local-first Git forge infrastructure.*

> [!CAUTION]
> This project is in active development.
> There are surely bugs and misbehaviors that have not yet been discovered.
> Please file a [new issue] for any misbehaviors you find!

[new issue]: https://github.com/git-ents/git-forge/issues/new

## Overview

This crate implements the issue tracking domain for `git-forge`.
Issues are stored as Git refs under `refs/meta/issues/`, with each mutation recorded as a new commit, giving every issue a built-in audit log.
Each issue ref points to a tree containing a TOML metadata file, a Markdown body, and a directory of conversation comments.

Issue comments are the conversation that takes place within an issue.
They are distinct from code comments, which are anchored to blob OIDs and managed by `git-forge-core`.

## Installation

The `git-forge-issues` library can be added to your Rust project via `cargo add`.

```shell
cargo add git-forge-issues
```
