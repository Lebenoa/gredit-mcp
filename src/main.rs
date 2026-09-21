use std::{env, io};

use anyhow::{Context, Result};
use gredit_mcp::FileSystemServer;
use rmcp::{ServiceExt, transport::stdio};
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env())
        .with_writer(io::stderr)
        .with_ansi(false)
        .init();

    let root = env::args_os()
        .nth(1)
        .map(std::path::PathBuf::from)
        .or_else(|| env::var_os("GREDIT_WORKSPACE").map(std::path::PathBuf::from))
        .unwrap_or(env::current_dir().context("failed to determine current directory")?);
    let server = FileSystemServer::new(&root)
        .with_context(|| format!("invalid workspace root: {}", root.display()))?;

    tracing::info!(root = %server.root().display(), "starting gredit MCP server");
    let service = server
        .serve(stdio())
        .await
        .context("failed to start MCP stdio service")?;
    service.waiting().await.context("MCP service failed")?;
    Ok(())
}
