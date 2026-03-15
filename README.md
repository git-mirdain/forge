# ⚒️ `git-forge`

*Local-first Git forge infrastructure.*

<!-- rumdl-disable MD013 -->
[![CI](https://github.com/git-ents/git-forge/actions/workflows/CI.yml/badge.svg)](https://github.com/git-ents/git-forge/actions/workflows/CI.yml)
[![CD](https://github.com/git-ents/git-forge/actions/workflows/CD.yml/badge.svg)](https://github.com/git-ents/git-forge/actions/workflows/CD.yml)
<!-- rumdl-enable MD013 -->

> [!CAUTION]
> This project is being actively developed!
> Despite this, semantic versioning rules will be respected.
> Expect frequent updates.

## About

To support a more expansive usage of the Git object database — as is the goal for other projects within the [`git-ents`](https://github.com/git-ents) organization — new tooling is needed.
This project aims to add support for local-first Git forge infrastructure: issues, reviews, and releases, all stored inside the repository itself.

You may see the terms *porcelain* and *plumbing* used across this project.
These are [borrowed from Git itself](https://git-scm.com/book/en/v2/Git-Internals-Plumbing-and-Porcelain): porcelain refers to user-facing commands, and plumbing refers to the lower-level libraries and commands they are built on.

## Crates

| Crate | Description | API |
|---|---|---|
| [`git-forge`](crates/git-forge/) | CLI entrypoint and library facade. | Porcelain |
| [`git-forge-core`](crates/git-forge-core/) | Shared annotations: code comments and approvals. | Plumbing |
| [`git-forge-issue`](crates/git-forge-issue/) | Issue tracking. | Porcelain |
| [`git-forge-review`](crates/git-forge-review/) | Pull/merge request reviews. | Porcelain |
| [`git-forge-release`](crates/git-forge-release/) | Release management. | Porcelain |
