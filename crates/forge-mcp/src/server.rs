//! MCP server struct and transport wiring.

use std::path::PathBuf;

use git2::Repository;
use rmcp::handler::server::router::tool::ToolRouter;
use rmcp::model::{ServerCapabilities, ServerInfo};
use rmcp::{ServerHandler, tool_handler};

/// MCP server that exposes forge metadata from a Git repository.
#[derive(Debug, Clone)]
pub struct ForgeMcpServer {
    repo_path: PathBuf,
    pub(crate) tool_router: ToolRouter<Self>,
}

impl ForgeMcpServer {
    /// Discover the nearest Git repository from the current directory.
    ///
    /// # Errors
    /// Returns an error if no repository is found.
    pub fn new() -> anyhow::Result<Self> {
        let repo = Repository::discover(".")?;
        let repo_path = repo.path().to_path_buf();
        Ok(Self {
            repo_path,
            tool_router: Self::issue_router(),
        })
    }

    pub(crate) fn open_repo(&self) -> anyhow::Result<Repository> {
        Ok(Repository::open(&self.repo_path)?)
    }
}

#[tool_handler]
impl ServerHandler for ForgeMcpServer {
    fn get_info(&self) -> ServerInfo {
        ServerInfo::new(ServerCapabilities::builder().enable_tools().build())
    }
}
