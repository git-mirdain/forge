//! Entity refs: issues and reviews.
//!
//! Entities are standalone refs with their own lifecycles. They are not
//! metadata on any object — each has its own commit history that serves as
//! its audit log.
//!
//! - [`issues`] — issue refs under `refs/meta/issues/`.
//! - [`reviews`] — review refs under `refs/meta/reviews/`.

pub mod issues;
pub mod reviews;
