//! Push enforcement: merge gate and queue primitive.
//!
//! - [`gate`] — evaluates whether a push satisfies all policy requirements.
//! - [`queue`] — general-purpose ordered queue refs, used by the merge queue.

pub mod gate;
pub mod queue;
