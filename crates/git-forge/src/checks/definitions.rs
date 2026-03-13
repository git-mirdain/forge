//! Check definitions read from `.forge/checks/`.
//!
//! Check definitions live in the repository, versioned with the code:
//!
//! ```text
//! .forge/checks/
//! ├── build.toml
//! ├── lint.toml
//! └── test.toml
//! ```
//!
//! The definition that runs is always the one at the commit being checked —
//! no external configuration that drifts from the code.

pub mod git2;

/// The working-tree path under which check definitions are stored.
pub const CHECKS_DEF_DIR: &str = ".forge/checks";

/// How a secret value is delivered into the container.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SecretKind {
    /// Written to a file under `/run/forge/secrets/<name>`.
    File,
}

/// A secret referenced by name in a check definition.
#[derive(Clone, Debug)]
pub struct SecretRef {
    /// The name of the secret (matches the server-side entry).
    pub name: String,
    /// How the secret is injected into the container.
    pub kind: SecretKind,
}

/// A parsed check definition from `.forge/checks/<name>.toml`.
#[derive(Clone, Debug)]
pub struct CheckDefinition {
    /// Canonical name of this check (e.g. `"build"`, `"test"`).
    pub name: String,
    /// Container image used to run the check (e.g. `"rust:1.85"`).
    pub image: String,
    /// Shell command executed inside the container.
    pub run: String,
    /// Ref patterns that trigger this check (e.g. `["refs/heads/*"]`).
    pub triggers: Vec<String>,
    /// Secrets injected into the container at run time.
    pub secrets: Vec<SecretRef>,
}

/// Operations for reading check definitions from the repository.
pub trait CheckDefinitions {
    /// Return all check definitions present in `.forge/checks/` at `HEAD`.
    fn list_check_definitions(&self) -> Result<Vec<CheckDefinition>, ::git2::Error>;

    /// Load the check definition named `name` from `.forge/checks/<name>.toml`
    /// at `HEAD`, returning `None` if it does not exist.
    fn find_check_definition(&self, name: &str) -> Result<Option<CheckDefinition>, ::git2::Error>;

    /// Load the check definition named `name` from the tree of `commit_oid`,
    /// returning `None` if it does not exist at that commit.
    ///
    /// The merge gate and runners use this to read the definition at the exact
    /// commit being checked, ensuring definitions stay in sync with the code.
    fn find_check_definition_at(
        &self,
        name: &str,
        commit_oid: ::git2::Oid,
    ) -> Result<Option<CheckDefinition>, ::git2::Error>;
}
