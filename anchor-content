//! GitHub import/export adapter for the forge store.

pub mod client;
pub mod config;
pub mod export;
pub mod import;
pub mod state;

use client::OctocrabClient;
use git_forge::sync::{RemoteSync, SyncReport};
use git_forge::{Error, Result};
use git2::Repository;

use crate::config::GitHubSyncConfig;

/// GitHub adapter that implements [`RemoteSync`].
pub struct GitHubAdapter {
    /// The sync configuration for this remote.
    pub config: GitHubSyncConfig,
}

impl GitHubAdapter {
    /// Create a new adapter from a sync configuration.
    #[must_use]
    pub fn new(config: GitHubSyncConfig) -> Self {
        Self { config }
    }
}

impl RemoteSync for GitHubAdapter {
    async fn import_issues(&self, repo: &Repository) -> Result<SyncReport> {
        let client = OctocrabClient::new(self.config.token.as_deref())
            .map_err(|e| Error::Sync(e.to_string()))?;
        import::import_issues(repo, &self.config, &client)
            .await
            .map_err(|e| Error::Sync(e.to_string()))
    }

    async fn export_issues(&self, repo: &Repository) -> Result<SyncReport> {
        let client = OctocrabClient::new(self.config.token.as_deref())
            .map_err(|e| Error::Sync(e.to_string()))?;
        export::export_issues(repo, &self.config, &client)
            .await
            .map_err(|e| Error::Sync(e.to_string()))
    }

    async fn import_reviews(&self, repo: &Repository) -> Result<SyncReport> {
        let client = OctocrabClient::new(self.config.token.as_deref())
            .map_err(|e| Error::Sync(e.to_string()))?;
        import::import_reviews(repo, &self.config, &client)
            .await
            .map_err(|e| Error::Sync(e.to_string()))
    }

    async fn export_reviews(&self, repo: &Repository) -> Result<SyncReport> {
        let client = OctocrabClient::new(self.config.token.as_deref())
            .map_err(|e| Error::Sync(e.to_string()))?;
        export::export_reviews(repo, &self.config, &client)
            .await
            .map_err(|e| Error::Sync(e.to_string()))
    }

    async fn import_all(&self, repo: &Repository) -> Result<SyncReport> {
        let client = OctocrabClient::new(self.config.token.as_deref())
            .map_err(|e| Error::Sync(e.to_string()))?;
        import::import_all(repo, &self.config, &client)
            .await
            .map_err(|e| Error::Sync(e.to_string()))
    }

    async fn export_all(&self, repo: &Repository) -> Result<SyncReport> {
        let client = OctocrabClient::new(self.config.token.as_deref())
            .map_err(|e| Error::Sync(e.to_string()))?;
        export::export_all(repo, &self.config, &client)
            .await
            .map_err(|e| Error::Sync(e.to_string()))
    }
}
