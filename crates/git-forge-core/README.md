# 🔩 `git-forge-core`

*Shared annotations for git-forge: code comments and approvals.*

> [!CAUTION]
> This project is in active development and has not yet been published to crates.io.
> Please file a [new issue] for any misbehaviors you find!

[new issue]: https://github.com/git-ents/git-forge/issues/new

## Overview

This crate provides the foundational annotation primitives used across the `git-forge` workspace.
Annotations are metadata attached directly to Git objects, independent of any issue or review that may have prompted them.

Comments are anchored to a blob OID and a line range, stored under `refs/metadata/comments/`.
They are repo-wide and persist as long as the code they describe exists — a review may prompt someone to leave a comment, but the comment itself is not owned by that review.

Approvals attest to correctness at four granularities: blob, tree, patch, and range-patch.
They are stored under `refs/metadata/approvals/` and keyed on patch-ids rather than commit OIDs, so approvals survive rebases automatically.

## Installation

The `git-forge-core` library can be added to your Rust project via `cargo add`.

```shell
cargo add --git https://github.com/git-ents/git-forge.git git-forge-core
