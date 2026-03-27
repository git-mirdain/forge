//! MCP server for exposing forge metadata from the Git object store.
//!
//! This crate provides both a library of MCP tool handlers and a binary
//! that runs the server over stdio.

mod issue;
mod server;

pub use server::ForgeMcpServer;
