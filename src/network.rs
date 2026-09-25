//! Network transports for the MCP server: Streamable HTTP (with SSE streaming)
//! and WebSocket. Each accepted connection gets its own server instance rooted
//! at the workspace path the process was started with.

use std::{io, net::SocketAddr, path::PathBuf, sync::Arc};

use subtle::ConstantTimeEq;

use anyhow::{Context, Result};
use axum::{
    Router,
    extract::State,
    http::{HeaderMap, StatusCode, header::AUTHORIZATION},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::get,
};
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
/// Network listeners are intentionally restricted to loopback. If a bearer
/// token is configured, requests must include it and `exec` is enabled.
pub async fn serve_http(
    root: PathBuf,
    addr: SocketAddr,
    bearer_token: Option<String>,
    allow_remote_exec: bool,
) -> Result<()> {
    ensure_loopback(addr)?;
    let allow_exec = allow_remote_exec;
    let session_manager = Arc::new(LocalSessionManager::default());
    let service = StreamableHttpService::new(
        move || {
            FileSystemServer::with_exec(&root, allow_exec)
                .map_err(|error| io::Error::other(error.to_string()))
        },
        session_manager,
        StreamableHttpServerConfig::default(),
    );
    let mut router = Router::new()
        .route("/", get(http_index))
        .nest_service("/mcp", service);
    if let Some(token) = bearer_token {
        router = router.layer(middleware::from_fn_with_state(token, require_bearer_token));
    }
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

async fn require_bearer_token(
    State(token): State<String>,
    headers: HeaderMap,
    request: axum::extract::Request,
    next: Next,
) -> Response {
    if headers
        .get(AUTHORIZATION)
        .is_some_and(|header| authorized(header.as_bytes(), &token))
    {
        next.run(request).await
    } else {
        (StatusCode::UNAUTHORIZED, "missing or invalid bearer token").into_response()
    }
}

fn authorized(header: &[u8], token: &str) -> bool {
    let expected = format!("Bearer {token}");
    bool::from(expected.as_bytes().ct_eq(header))
}

async fn http_index() -> &'static str {
    "gredit-mcp: MCP endpoint at /mcp\n"
}

/// Serve the MCP protocol over WebSocket, one MCP session per connection.
///
/// Each WebSocket text or binary frame carries one JSON-RPC message. Network
/// listeners are intentionally restricted to loopback. A configured bearer
/// token is required in the upgrade request and enables `exec`.
pub async fn serve_ws(
    root: PathBuf,
    addr: SocketAddr,
    bearer_token: Option<String>,
    allow_remote_exec: bool,
) -> Result<()> {
    ensure_loopback(addr)?;
    let allow_exec = allow_remote_exec;
    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .with_context(|| format!("failed to bind WebSocket listener on {addr}"))?;
    tracing::info!(%addr, "MCP WebSocket server listening at ws://{addr}");

    loop {
        let (stream, peer) = listener.accept().await?;
        let root = root.clone();
        let bearer_token = bearer_token.clone();
        tokio::spawn(async move {
            if let Err(error) = handle_ws_connection(root, stream, bearer_token, allow_exec).await {
                tracing::warn!(%peer, %error, "WebSocket connection error");
            }
        });
    }
}

struct WebSocketBearerAuth(String);

impl tokio_tungstenite::tungstenite::handshake::server::Callback for WebSocketBearerAuth {
    #[allow(clippy::result_large_err)]
    fn on_request(
        self,
        request: &tokio_tungstenite::tungstenite::handshake::server::Request,
        response: tokio_tungstenite::tungstenite::handshake::server::Response,
    ) -> Result<
        tokio_tungstenite::tungstenite::handshake::server::Response,
        tokio_tungstenite::tungstenite::handshake::server::ErrorResponse,
    > {
        if request
            .headers()
            .get(AUTHORIZATION)
            .is_some_and(|header| authorized(header.as_bytes(), &self.0))
        {
            Ok(response)
        } else {
            Err(tokio_tungstenite::tungstenite::http::Response::builder()
                .status(StatusCode::UNAUTHORIZED)
                .body(Some("missing or invalid bearer token".to_owned()))
                .expect("valid unauthorized response"))
        }
    }
}

async fn handle_ws_connection(
    root: PathBuf,
    stream: tokio::net::TcpStream,
    bearer_token: Option<String>,
    allow_exec: bool,
) -> Result<()> {
    let socket = if let Some(token) = bearer_token {
        tokio_tungstenite::accept_hdr_async(stream, WebSocketBearerAuth(token)).await?
    } else {
        tokio_tungstenite::accept_async(stream).await?
    };
    let server = FileSystemServer::with_exec(&root, allow_exec)?;
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
