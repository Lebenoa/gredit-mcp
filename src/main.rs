use std::{env, io, net::SocketAddr, path::PathBuf};

use anyhow::{Context, Result};
use gredit_mcp::{FileSystemServer, network};
use rmcp::{ServiceExt, transport::stdio};
use tracing_subscriber::EnvFilter;

const DEFAULT_HTTP_ADDR: &str = "127.0.0.1:3000";
const DEFAULT_WS_ADDR: &str = "127.0.0.1:8080";

#[derive(Clone, Copy, PartialEq)]
enum Transport {
    Stdio,
    Http,
    Ws,
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env())
        .with_writer(io::stderr)
        .with_ansi(false)
        .init();

    let mut transport = Transport::Stdio;
    let mut addr: Option<SocketAddr> = None;
    let mut root: Option<PathBuf> = None;

    let mut args = env::args_os().skip(1);
    while let Some(arg) = args.next() {
        let text = arg.to_string_lossy();
        match text.as_ref() {
            "--http" => transport = Transport::Http,
            "--ws" => transport = Transport::Ws,
            "--addr" => {
                let value = args
                    .next()
                    .context("--addr requires an address, e.g. 127.0.0.1:3000")?;
                addr = Some(value.to_string_lossy().parse()?);
            }
            _ if text.starts_with("--addr=") => {
                addr = Some(text["--addr=".len()..].parse()?);
            }
            _ if text.starts_with('-') => return Err(anyhow::anyhow!("unknown flag: {text}")),
            _ => root = Some(PathBuf::from(&arg)),
        }
    }

    let root = root
        .or_else(|| env::var_os("GREDIT_WORKSPACE").map(PathBuf::from))
        .unwrap_or(env::current_dir().context("failed to determine current directory")?);
    let server = FileSystemServer::new(&root)
        .with_context(|| format!("invalid workspace root: {}", root.display()))?;
    tracing::info!(root = %server.root().display(), "starting gredit MCP server");

    match transport {
        Transport::Stdio => {
            let service = server
                .serve(stdio())
                .await
                .context("failed to start MCP stdio service")?;
            service.waiting().await.context("MCP service failed")?;
        }
        Transport::Http => {
            let addr = addr.unwrap_or_else(|| DEFAULT_HTTP_ADDR.parse().expect("static addr"));
            network::serve_http(root, addr).await?;
        }
        Transport::Ws => {
            let addr = addr.unwrap_or_else(|| DEFAULT_WS_ADDR.parse().expect("static addr"));
            network::serve_ws(root, addr).await?;
        }
    }
    Ok(())
}
