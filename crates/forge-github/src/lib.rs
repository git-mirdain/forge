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
    /// Cached API client, constructed once from `config.token`.
    client: OctocrabClient,
}

impl GitHubAdapter {
    /// Create a new adapter from a sync configuration.
    ///
    /// # Errors
    /// Returns an error if the API client cannot be constructed (e.g. missing token).
    pub fn new(config: GitHubSyncConfig) -> Result<Self> {
        let client =
            OctocrabClient::new(config.token.as_deref()).map_err(|e| Error::Sync(e.to_string()))?;
        Ok(Self { config, client })
    }
}

impl RemoteSync for GitHubAdapter {
    async fn import_issues(&self, repo: &Repository) -> Result<SyncReport> {
        import::import_issues(repo, &self.config, &self.client)
            .await
            .map_err(|e| Error::Sync(e.to_string()))
    }

    async fn export_issues(&self, repo: &Repository) -> Result<SyncReport> {
        export::export_issues(repo, &self.config, &self.client)
            .await
            .map_err(|e| Error::Sync(e.to_string()))
    }

    async fn import_reviews(&self, repo: &Repository) -> Result<SyncReport> {
        import::import_reviews(repo, &self.config, &self.client)
            .await
            .map_err(|e| Error::Sync(e.to_string()))
    }

    async fn export_reviews(&self, repo: &Repository) -> Result<SyncReport> {
        export::export_reviews(repo, &self.config, &self.client)
            .await
            .map_err(|e| Error::Sync(e.to_string()))
    }

    async fn import_all(&self, repo: &Repository) -> Result<SyncReport> {
        import::import_all(repo, &self.config, &self.client)
            .await
            .map_err(|e| Error::Sync(e.to_string()))
    }

    async fn export_all(&self, repo: &Repository) -> Result<SyncReport> {
        export::export_all(repo, &self.config, &self.client)
            .await
            .map_err(|e| Error::Sync(e.to_string()))
    }
}
