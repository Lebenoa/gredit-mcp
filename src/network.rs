//! Network transports for the MCP server: Streamable HTTP (with SSE streaming)
//! and WebSocket. Each accepted connection gets its own server instance rooted
//! at the workspace path the process was started with.

use std::{io, net::SocketAddr, path::PathBuf, sync::Arc};

use anyhow::{Context, Result};
use axum::{Router, routing::get};
use futures_util::{SinkExt, StreamExt};
use rmcp::{
    RoleServer,
    service::{RxJsonRpcMessage, TxJsonRpcMessage},
    transport::{
        Transport,
        streamable_http_server::{
            StreamableHttpServerConfig, StreamableHttpService, session::local::LocalSessionManager,
        },
    },
};
use tokio_tungstenite::tungstenite::Message;

use crate::workspace::FileSystemServer;

/// Serve the MCP protocol over Streamable HTTP plus the SSE stream endpoint.
///
/// Both the JSON-RPC `POST /mcp` endpoint and the `GET /mcp` SSE stream are
/// provided by rmcp's `StreamableHttpService`. Each session is created from
/// the workspace root via [`FileSystemServer::new`], so `set_workspace`
/// approvals are per-session.
///
/// Network listeners are intentionally restricted to loopback. Put an
/// authenticated reverse proxy in front when remote access is required.
pub async fn serve_http(root: PathBuf, addr: SocketAddr) -> Result<()> {
    ensure_loopback(addr)?;
    let session_manager = Arc::new(LocalSessionManager::default());
    let service = StreamableHttpService::new(
        move || FileSystemServer::new(&root).map_err(|error| io::Error::other(error.to_string())),
        session_manager,
        StreamableHttpServerConfig::default(),
    );
    let router = Router::new()
        .route("/", get(http_index))
        .nest_service("/mcp", service);
    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .with_context(|| format!("failed to bind HTTP listener on {addr}"))?;
    tracing::info!(%addr, "MCP Streamable HTTP/SSE server listening at http://{addr}/mcp");

    axum::serve(listener, router)
        .with_graceful_shutdown(async {
            let _ = tokio::signal::ctrl_c().await;
        })
        .await
        .context("HTTP server failed")
}

async fn http_index() -> &'static str {
    "gredit-mcp: MCP endpoint at /mcp\n"
}

/// Serve the MCP protocol over WebSocket, one MCP session per connection.
///
/// Each WebSocket text or binary frame carries one JSON-RPC message.
/// Network listeners are intentionally restricted to loopback.
pub async fn serve_ws(root: PathBuf, addr: SocketAddr) -> Result<()> {
    ensure_loopback(addr)?;
    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .with_context(|| format!("failed to bind WebSocket listener on {addr}"))?;
    tracing::info!(%addr, "MCP WebSocket server listening at ws://{addr}");

    loop {
        let (stream, peer) = listener.accept().await?;
        let root = root.clone();
        tokio::spawn(async move {
            if let Err(error) = handle_ws_connection(root, stream).await {
                tracing::warn!(%peer, %error, "WebSocket connection error");
            }
        });
    }
}

async fn handle_ws_connection(root: PathBuf, stream: tokio::net::TcpStream) -> Result<()> {
    let socket = tokio_tungstenite::accept_async(stream).await?;
    let server = FileSystemServer::new(&root)?;
    let running = rmcp::serve_server(server, WsTransport::new(socket))
        .await
        .context("failed to start WebSocket MCP session")?;
    running
        .waiting()
        .await
        .context("WebSocket MCP session failed")?;
    Ok(())
}

/// Bridges a WebSocket connection to an rmcp [`Transport`]: each JSON-RPC
/// message travels in one WebSocket text frame.
struct WsTransport {
    sink: Arc<
        tokio::sync::Mutex<
            futures_util::stream::SplitSink<
                tokio_tungstenite::WebSocketStream<tokio::net::TcpStream>,
                Message,
            >,
        >,
    >,
    stream: futures_util::stream::SplitStream<
        tokio_tungstenite::WebSocketStream<tokio::net::TcpStream>,
    >,
}

impl WsTransport {
    fn new(socket: tokio_tungstenite::WebSocketStream<tokio::net::TcpStream>) -> Self {
        let (sink, stream) = socket.split();
        Self {
            sink: Arc::new(tokio::sync::Mutex::new(sink)),
            stream,
        }
    }
}

impl Transport<RoleServer> for WsTransport {
    type Error = tokio_tungstenite::tungstenite::Error;

    // The trait requires a `Send + 'static` future; an `async fn` borrows
    // `&mut self`, so the future is built by hand.
    #[allow(clippy::manual_async_fn)]
    fn send(
        &mut self,
        item: TxJsonRpcMessage<RoleServer>,
    ) -> impl std::future::Future<Output = Result<(), Self::Error>> + Send + 'static {
        let sink = self.sink.clone();
        async move {
            let text = serde_json::to_string(&item).map_err(serde_json_error)?;
            let mut sink = sink.lock().await;
            sink.send(Message::Text(text.into())).await
        }
    }

    async fn receive(&mut self) -> Option<RxJsonRpcMessage<RoleServer>> {
        loop {
            let message = match self.stream.next().await {
                Some(message) => message,
                None => return None,
            };
            let message = match message {
                Ok(message) => message,
                Err(_) => continue,
            };
            match message {
                Message::Text(text) => {
                    let parsed = serde_json::from_str::<RxJsonRpcMessage<RoleServer>>(&text);
                    if let Ok(message) = parsed {
                        return Some(message);
                    }
                }
                Message::Binary(binary) => {
                    let parsed = serde_json::from_slice::<RxJsonRpcMessage<RoleServer>>(&binary);
                    if let Ok(message) = parsed {
                        return Some(message);
                    }
                }
                _ => {}
            }
        }
    }

    async fn close(&mut self) -> Result<(), Self::Error> {
        let mut sink = self.sink.lock().await;
        sink.close().await
    }
}

fn ensure_loopback(addr: SocketAddr) -> Result<()> {
    if addr.ip().is_loopback() {
        Ok(())
    } else {
        Err(anyhow::anyhow!(
            "network transports only bind loopback addresses; use an authenticated reverse proxy or SSH tunnel for remote access"
        ))
    }
}

fn serde_json_error(error: serde_json::Error) -> tokio_tungstenite::tungstenite::Error {
    tokio_tungstenite::tungstenite::Error::Io(io::Error::other(error))
}
