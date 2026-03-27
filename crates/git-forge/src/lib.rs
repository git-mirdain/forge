//! Local-first infrastructure for Git forges.

pub mod issue;
pub mod refs;

mod index;
mod tree;

#[cfg(test)]
mod tests;
