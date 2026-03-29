# `git-forge`

*Local-first Git forge infrastructure.*

<!-- rumdl-disable MD013 -->
[![CI](https://github.com/git-mirdain/forge/actions/workflows/CI.yml/badge.svg)](https://github.com/git-mirdain/forge/actions/workflows/CI.yml)
[![CD](https://github.com/git-mirdain/forge/actions/workflows/CD.yml/badge.svg)](https://github.com/git-mirdain/forge/actions/workflows/CD.yml)
<!-- rumdl-enable MD013 -->

> [!CAUTION]
> This project is being actively developed!
> Despite this, semantic versioning rules will be respected.
> Expect frequent updates.

## About

`git-forge` provides local-first forge infrastructure — issues, reviews, and releases — stored inside the repository itself using the Git object database.

You may see the terms *porcelain* and *plumbing* used across this project.
These are [borrowed from Git itself](https://git-scm.com/book/en/v2/Git-Internals-Plumbing-and-Porcelain): porcelain refers to user-facing commands, and plumbing refers to the lower-level libraries and commands they are built on.

## Crates

| Crate | Description | API |
|---|---|---|
| [`git-forge`](crates/git-forge/) | CLI entrypoint and library facade. | Porcelain |
| [`forge-github`](crates/forge-github/) | GitHub import adapter for the forge store. | Plumbing |
| [`forge-mcp`](crates/forge-mcp/) | MCP server exposing forge metadata from the Git object store. | Plumbing |
| [`forge-server`](crates/forge-server/) | Sync daemon — watches refs and coordinates GitHub sync. | Plumbing |
| [`forge-zed`](extensions/forge-zed/) | Zed editor extension — surfaces forge tools via MCP context server. | Plumbing |
