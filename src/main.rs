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
    let mut allow_remote_exec = false;
    let mut root: Option<PathBuf> = None;

    let mut args = env::args_os().skip(1);
    while let Some(arg) = args.next() {
        let text = arg.to_string_lossy();
        match text.as_ref() {
            "--http" => transport = Transport::Http,
            "--ws" => transport = Transport::Ws,
            "--allow-remote-exec" => allow_remote_exec = true,
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
    if allow_remote_exec && transport == Transport::Stdio {
        anyhow::bail!("--allow-remote-exec is only valid with --http or --ws");
    }

    let server = match transport {
        Transport::Stdio => FileSystemServer::new_with_exec(&root),
        Transport::Http | Transport::Ws => FileSystemServer::with_exec(&root, false),
    }
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
            let token = network_bearer_token()?;
            if allow_remote_exec {
                tracing::warn!(
                    "remote command execution is enabled for network clients; listener is unauthenticated unless GREDIT_MCP_BEARER_TOKEN is set"
                );
            } else if token.is_some() {
                tracing::warn!("bearer authentication is not encrypted without TLS");
            }
            network::serve_http(root, addr, token, allow_remote_exec).await?;
        }
        Transport::Ws => {
            let addr = addr.unwrap_or_else(|| DEFAULT_WS_ADDR.parse().expect("static addr"));
            let token = network_bearer_token()?;
            if allow_remote_exec {
                tracing::warn!(
                    "remote command execution is enabled for network clients; listener is unauthenticated unless GREDIT_MCP_BEARER_TOKEN is set"
                );
            } else if token.is_some() {
                tracing::warn!("bearer authentication is not encrypted without TLS");
            }
            network::serve_ws(root, addr, token, allow_remote_exec).await?;
        }
    }
    Ok(())
}

fn network_bearer_token() -> Result<Option<String>> {
    let Some(token) = env::var_os("GREDIT_MCP_BEARER_TOKEN") else {
        return Ok(None);
    };
    let token = token
        .into_string()
        .map_err(|_| anyhow::anyhow!("GREDIT_MCP_BEARER_TOKEN must be valid Unicode"))?;
    if token.is_empty() {
        anyhow::bail!("GREDIT_MCP_BEARER_TOKEN must not be empty");
    }
    Ok(Some(token))
}
