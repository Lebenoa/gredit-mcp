use std::{collections::HashMap, fs, net::SocketAddr, time::Duration};

use rmcp::handler::server::wrapper::Parameters;
use tempfile::tempdir;

use crate::{
    EditRequest, ExecRequest, FileSystemServer, GrepRequest, ReadRequest, WriteRequest, network,
};

fn server() -> (tempfile::TempDir, FileSystemServer) {
    let directory = tempdir().expect("temp directory");
    let server = FileSystemServer::new(directory.path()).expect("server");
    (directory, server)
}

fn trusted_server() -> (tempfile::TempDir, FileSystemServer) {
    let directory = tempdir().expect("temp directory");
    let server = FileSystemServer::new_with_exec(directory.path()).expect("trusted server");
    (directory, server)
}

/// Unwrap a tool result's `Json<T>` payload, since `Json<T>` is not `Debug`.
fn expect_ok<T>(result: Result<rmcp::Json<T>, rmcp::ErrorData>, message: &str) -> T {
    match result {
        Ok(rmcp::Json(value)) => value,
        Err(error) => panic!("{message}: {error}"),
    }
}

/// Extract the error from a tool result, panicking on success.
fn expect_err<T>(result: Result<T, rmcp::ErrorData>, message: &str) -> rmcp::ErrorData {
    match result {
        Ok(_) => panic!("{message}: expected an error, got Ok"),
        Err(error) => error,
    }
}

#[test]
fn rejects_absolute_and_parent_paths() {
    let (_directory, server) = server();
    assert!(server.resolve_existing("../outside").is_err());
    assert!(server.resolve_existing("C:/outside").is_err());
    assert!(server.resolve_existing("/outside").is_err());
}

#[test]
fn edit_requires_one_match_unless_replace_all() {
    let (_directory, server) = server();
    let file = server.root().join("example.txt");
    fs::write(&file, "one\none\n").expect("write fixture");

    let error = expect_err(
        server.edit(Parameters(EditRequest {
            path: "example.txt".to_owned(),
            old_string: "one".to_owned(),
            new_string: "two".to_owned(),
            replace_all: None,
        })),
        "ambiguous edit should fail",
    );
    assert!(error.message.contains("matched 2 times"));

    let result = expect_ok(
        server.edit(Parameters(EditRequest {
            path: "example.txt".to_owned(),
            old_string: "one".to_owned(),
            new_string: "two".to_owned(),
            replace_all: Some(true),
        })),
        "replace all",
    );
    assert_eq!(result.replacements, 2);
    assert_eq!(
        fs::read_to_string(file).expect("read fixture"),
        "two\ntwo\n"
    );
}

#[tokio::test]
async fn exec_is_disabled_on_workspace_only_server() {
    let (_directory, server) = server();
    let error = expect_err(
        server
            .exec(Parameters(ExecRequest {
                command: "echo hello".to_owned(),
                working_dir: None,
                env: None,
                timeout_ms: Some(5_000),
                max_output_bytes: Some(1_024),
            }))
            .await,
        "exec should be disabled",
    );
    assert!(error.message.contains("exec is disabled"));
}

#[tokio::test]
async fn exec_runs_in_workspace_with_embedded_nushell() {
    let (_directory, server) = trusted_server();
    let result = expect_ok(
        server
            .exec(Parameters(ExecRequest {
                command: "echo hello".to_owned(),
                working_dir: None,
                env: None,
                timeout_ms: Some(5_000),
                max_output_bytes: Some(1_024),
            }))
            .await,
        "exec",
    );
    assert_eq!(result.engine, "embedded-nushell");
    assert_eq!(result.exit_code, 0);
    assert!(result.stdout.contains("hello"));
    assert!(!result.output_truncated);
}

#[tokio::test]
async fn exec_evaluates_nushell_pipeline() {
    let (_directory, server) = trusted_server();
    let result = expect_ok(
        server
            .exec(Parameters(ExecRequest {
                command: "[3 1 2] | sort | str join ','".to_owned(),
                working_dir: None,
                env: None,
                timeout_ms: Some(5_000),
                max_output_bytes: Some(1_024),
            }))
            .await,
        "pipeline exec",
    );
    assert!(result.stdout.contains("1,2,3"));
}

#[tokio::test]
async fn exec_rejects_outside_working_directory() {
    let (_directory, server) = trusted_server();
    let error = expect_err(
        server
            .exec(Parameters(ExecRequest {
                command: "echo hello".to_owned(),
                working_dir: Some("../".to_owned()),
                env: None,
                timeout_ms: None,
                max_output_bytes: None,
            }))
            .await,
        "outside working directory should fail",
    );
    assert!(error.message.contains(".."));
}

#[test]
fn write_read_and_grep_work_inside_root() {
    let (_directory, server) = server();
    let file = server.root().join("notes.txt");
    fs::write(&file, "alpha\nbeta\n").expect("write fixture");

    let written = expect_ok(
        server.write(Parameters(WriteRequest {
            path: "notes.txt".to_owned(),
            content: "one\ntwo\n".to_owned(),
            create_dirs: None,
        })),
        "write",
    );
    assert!(written.path.contains("notes.txt"));

    let read = expect_ok(
        server.read(Parameters(ReadRequest {
            path: "notes.txt".to_owned(),
            start_line: None,
            end_line: None,
            max_bytes: None,
        })),
        "read",
    );
    assert_eq!(read.total_lines, 2);
    assert_eq!(read.lines[0].number, 1);
    assert_eq!(read.lines[0].text, "one");
    assert_eq!(read.lines[1].text, "two");

    let found = expect_ok(
        server.grep(Parameters(GrepRequest {
            query: "^two".to_owned(),
            path: Some(".".to_owned()),
            file_suffix: None,
            max_results: None,
            case_insensitive: None,
        })),
        "grep",
    );
    assert_eq!(found.matches.len(), 1);
    assert!(found.matches[0].path.contains("notes.txt"));
    assert!(found.matches[0].text.starts_with("two"));
}

// ─── Network transport smoke tests ─────────────────────────────────────────

fn free_addr() -> SocketAddr {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind");
    listener.local_addr().expect("local addr")
}

async fn http_request(
    addr: SocketAddr,
    method: &str,
    path: &str,
    extra_headers: &[(&str, &str)],
    body: &str,
) -> (u16, HashMap<String, String>, String) {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    let mut stream = tokio::time::timeout(
        Duration::from_secs(10),
        tokio::net::TcpStream::connect(addr),
    )
    .await
    .expect("connect timeout")
    .expect("connect");

    let mut request = format!("{method} {path} HTTP/1.1\r\nHost: {addr}\r\nConnection: close\r\n");
    request.push_str("Accept: application/json, text/event-stream\r\n");
    if !body.is_empty() {
        request.push_str("Content-Type: application/json\r\n");
    }
    for (name, value) in extra_headers {
        request.push_str(&format!("{name}: {value}\r\n"));
    }
    request.push_str(&format!("Content-Length: {}\r\n\r\n", body.len()));
    stream
        .write_all(request.as_bytes())
        .await
        .expect("write request");
    stream.write_all(body.as_bytes()).await.expect("write body");

    // Read the response head up to the blank line.
    let mut raw = Vec::new();
    let mut byte = [0u8; 1];
    while !raw.ends_with(b"\r\n\r\n") {
        match tokio::time::timeout(Duration::from_secs(10), stream.read(&mut byte)).await {
            Ok(Ok(0)) => break,
            Ok(Ok(_)) => raw.push(byte[0]),
            _ => break,
        }
    }
    let text = String::from_utf8_lossy(&raw);
    let text = text.replace("\r\n", "\n");
    let (head, _rest) = text.split_once("\n\n").unwrap_or((&text, ""));
    let mut lines = head.lines();
    let status: u16 = lines
        .next()
        .and_then(|line| line.split_whitespace().nth(1))
        .and_then(|status| status.parse().ok())
        .expect("status line");
    let headers: HashMap<String, String> = lines
        .filter_map(|line| line.split_once(':'))
        .map(|(key, value)| (key.trim().to_ascii_lowercase(), value.trim().to_owned()))
        .collect();

    // Read the declared body if any; otherwise drain briefly (SSE streams stay
    // open, so never wait for EOF on those).
    let body = match headers
        .get("content-length")
        .and_then(|length| length.parse::<usize>().ok())
    {
        Some(length) => {
            let mut rest = vec![0u8; length];
            let mut filled = 0;
            while filled < length {
                let n =
                    tokio::time::timeout(Duration::from_secs(10), stream.read(&mut rest[filled..]))
                        .await
                        .expect("body read timeout")
                        .expect("body read");
                filled += n;
            }
            String::from_utf8_lossy(&rest).into_owned()
        }
        None => {
            let mut rest = Vec::new();
            let mut byte = [0u8; 1];
            while rest.len() < 65_536 {
                match tokio::time::timeout(Duration::from_millis(300), stream.read(&mut byte)).await
                {
                    Ok(Ok(0)) => break,
                    Ok(Ok(_)) => rest.push(byte[0]),
                    _ => break,
                }
            }
            String::from_utf8_lossy(&rest).into_owned()
        }
    };
    (status, headers, body)
}

const INITIALIZE: &str = r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-03-26","capabilities":{},"clientInfo":{"name":"smoke-test","version":"0.0.0"}}}"#;

#[tokio::test]
async fn streamable_http_serves_initialize() {
    let directory = tempdir().expect("temp directory");
    let addr = free_addr();
    let handle = tokio::spawn(network::serve_http(directory.path().to_path_buf(), addr));
    tokio::time::sleep(Duration::from_millis(100)).await;

    let (status, headers, body) = http_request(addr, "POST", "/mcp", &[], INITIALIZE).await;
    assert_eq!(status, 200, "unexpected response: {body}");
    assert!(headers.contains_key("mcp-session-id"));
    assert!(body.contains("\"serverInfo\""));
    assert!(body.contains("\"name\""));

    handle.abort();
}

#[tokio::test]
async fn streamable_http_sse_get_streams() {
    let directory = tempdir().expect("temp directory");
    let addr = free_addr();
    let handle = tokio::spawn(network::serve_http(directory.path().to_path_buf(), addr));
    tokio::time::sleep(Duration::from_millis(100)).await;

    let (status, headers, _body) = http_request(addr, "POST", "/mcp", &[], INITIALIZE).await;
    assert_eq!(status, 200);
    let session_id = headers
        .get("mcp-session-id")
        .cloned()
        .expect("session id header");

    // The SSE stream is tied to an established session, so replay it here.
    let stream_headers = [
        ("MCP-Session-Id", session_id.as_str()),
        ("Last-Event-ID", "0"),
    ];
    let (status, headers, _body) = http_request(addr, "GET", "/mcp", &stream_headers, "").await;
    assert_eq!(status, 200, "unexpected SSE response");
    assert_eq!(
        headers.get("content-type").map(String::as_str),
        Some("text/event-stream")
    );

    handle.abort();
}

#[tokio::test]
async fn websocket_serves_initialize() {
    use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    let directory = tempdir().expect("temp directory");
    let addr = free_addr();
    let handle = tokio::spawn(network::serve_ws(directory.path().to_path_buf(), addr));
    tokio::time::sleep(Duration::from_millis(100)).await;

    let mut stream = tokio::time::timeout(
        Duration::from_secs(10),
        tokio::net::TcpStream::connect(addr),
    )
    .await
    .expect("connect timeout")
    .expect("connect");

    let key = BASE64.encode([0xAB; 16]);
    let upgrade = format!(
        "GET / HTTP/1.1\r\nHost: {addr}\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Key: {key}\r\nSec-WebSocket-Version: 13\r\n\r\n"
    );
    stream
        .write_all(upgrade.as_bytes())
        .await
        .expect("upgrade write");
    let mut buf = [0u8; 1024];
    let n = stream.read(&mut buf).await.expect("upgrade read");
    let response = String::from_utf8_lossy(&buf[..n]);
    assert!(
        response.starts_with("HTTP/1.1 101"),
        "handshake: {response}"
    );

    // Send a masked text frame with the initialize message.
    let body = INITIALIZE.as_bytes();
    let mask = [0x11, 0x22, 0x33, 0x44];
    let masked: Vec<u8> = body
        .iter()
        .enumerate()
        .map(|(index, byte)| byte ^ mask[index % 4])
        .collect();
    let mut frame = vec![0x81];
    let length = body.len();
    if length < 126 {
        frame.push(0x80 | length as u8);
    } else if length <= u16::MAX as usize {
        frame.push(0x80 | 126);
        frame.extend_from_slice(&(length as u16).to_be_bytes());
    } else {
        frame.push(0x80 | 127);
        frame.extend_from_slice(&(length as u64).to_be_bytes());
    }
    frame.extend_from_slice(&mask);
    frame.extend_from_slice(&masked);
    stream.write_all(&frame).await.expect("frame write");

    // Read the server's text message, reassembling any fragmented frames.
    let mut payload = Vec::new();
    loop {
        let mut header = [0u8; 2];
        stream.read_exact(&mut header).await.expect("frame header");
        assert_eq!(header[0] & 0x0F, 0x01, "expected text frame");
        let fin = header[0] & 0x80 != 0;
        let masked = header[1] & 0x80 != 0;
        let length = match header[1] & 0x7F {
            126 => {
                let mut wide = [0u8; 2];
                stream.read_exact(&mut wide).await.expect("frame length");
                u16::from_be_bytes(wide) as usize
            }
            127 => {
                let mut wide = [0u8; 8];
                stream.read_exact(&mut wide).await.expect("frame length");
                u64::from_be_bytes(wide) as usize
            }
            length => length as usize,
        };
        let mut mask = [0u8; 4];
        if masked {
            stream.read_exact(&mut mask).await.expect("frame mask");
        }
        let mut chunk = vec![0u8; length];
        stream.read_exact(&mut chunk).await.expect("frame body");
        if masked {
            for (index, byte) in chunk.iter_mut().enumerate() {
                *byte ^= mask[index % 4];
            }
        }
        payload.extend_from_slice(&chunk);
        if fin {
            break;
        }
    }
    let text = String::from_utf8(payload).expect("utf8 frame");
    assert!(text.contains("\"serverInfo\""));
    assert!(text.contains("\"name\""));

    handle.abort();
}
