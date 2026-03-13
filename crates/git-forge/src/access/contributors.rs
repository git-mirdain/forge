//! Contributor records under `refs/meta/contributors`.
//!
//! Contributors are stored in a ref:
//!
//! ```text
//! refs/meta/contributors → commit → tree
//! ├── <fingerprint>/
//! │   ├── key.pub     # SSH or GPG public key
//! │   ├── meta        # toml: name, email, added_by, timestamp
//! │   └── roles       # toml: list of role names
//! ```
//!
//! Adding a contributor is a signed commit to this ref. The commit must be
//! signed by someone with `modify_contributors` permission. The first commit
//! is self-signed by the project creator — this bootstraps trust.

pub mod git2;

/// The ref under which contributor records are stored.
pub const CONTRIBUTORS_REF: &str = "refs/meta/contributors";

/// A contributor record loaded from `refs/meta/contributors/<fingerprint>/`.
#[derive(Clone, Debug)]
pub struct Contributor {
    /// The contributor's key fingerprint; used as the stable identifier.
    pub fingerprint: String,
    /// Raw public key bytes (SSH or GPG).
    pub public_key: Vec<u8>,
    /// Display name.
    pub name: String,
    /// Email address.
    pub email: String,
    /// Fingerprint of the contributor who added this entry.
    pub added_by: String,
    /// RFC 3339 timestamp of when this contributor was added.
    pub timestamp: String,
    /// Role names assigned to this contributor.
    pub roles: Vec<String>,
}

/// Parameters for adding a new contributor.
#[derive(Clone, Debug)]
pub struct NewContributor {
    /// Raw public key bytes.
    pub public_key: Vec<u8>,
    /// Display name.
    pub name: String,
    /// Email address.
    pub email: String,
    /// Initial role names (may be empty).
    pub roles: Vec<String>,
}

/// Operations on contributor records stored under [`CONTRIBUTORS_REF`].
pub trait Contributors {
    /// Return all contributors, ordered by fingerprint.
    fn list_contributors(&self) -> Result<Vec<Contributor>, ::git2::Error>;

    /// Load the contributor identified by `fingerprint`, returning `None` if
    /// they are not in the contributors ref.
    fn find_contributor(&self, fingerprint: &str) -> Result<Option<Contributor>, ::git2::Error>;

    /// Add a new contributor. The calling code is responsible for signing the
    /// resulting commit with a key that has `modify_contributors` permission.
    fn add_contributor(&self, contributor: &NewContributor) -> Result<(), ::git2::Error>;

    /// Remove the contributor identified by `fingerprint`.
    fn remove_contributor(&self, fingerprint: &str) -> Result<(), ::git2::Error>;

    /// Replace the full role list for the contributor identified by
    /// `fingerprint`.
    fn set_contributor_roles(
        &self,
        fingerprint: &str,
        roles: &[String],
    ) -> Result<(), ::git2::Error>;

    /// Return `true` if `fingerprint` holds any of the named `roles`.
    fn contributor_has_role(
        &self,
        fingerprint: &str,
        roles: &[&str],
    ) -> Result<bool, ::git2::Error>;
}
