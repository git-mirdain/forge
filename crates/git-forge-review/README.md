# 🔍 `git-forge-review`

*Pull/merge request reviews for local-first Git forge infrastructure.*

> [!CAUTION]
> This project is in active development and has not yet been published to <crates.io>.
> Please file a [new issue] for any misbehaviors you find!

[new issue]: https://github.com/git-ents/git-forge/issues/new

## Overview

This crate implements the review domain for `git-forge`.
A review is a coordination entity — "please look at commits X..Y" — stored as a Git ref under `refs/meta/reviews/`.
Each mutation is recorded as a new commit on the review's ref, so the commit history serves as the review's audit log.

A review does not contain comments or approvals; it prompts them.
Comments land on blob OIDs via `git-forge-core`, and approvals land on patch-ids and OIDs via `git-forge-core`.
The review is how you discover which commits to look at; the annotations are what you find when you look.

## Installation

The `git-forge-review` library can be added to your Rust project via `cargo add`.

```shell
cargo add --git https://github.com/git-ents/git-forge.git git-forge-review
```
