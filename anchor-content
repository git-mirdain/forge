//! Forge MCP server binary — runs over stdio.

use forge_mcp::ForgeMcpServer;
use rmcp::ServiceExt;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let server = ForgeMcpServer::new()?;
    let service = server.serve(rmcp::transport::io::stdio()).await?;
    service.waiting().await?;
    Ok(())
}
