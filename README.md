# gredit-mcp

A Model Context Protocol (MCP) server for inspecting and editing a project workspace. It exposes structured file tools, ripgrep-powered search, and an embedded Nushell runner for trusted clients.

- **Read, write, and edit** UTF-8 files without leaving the MCP client.
- **List and search** workspace files; search respects `.gitignore` and standard ripgrep filters.
- **Run Nushell** in the workspace when the client connection is trusted.
- **Use stdio, Streamable HTTP, or WebSocket** transports.

## Install

Download the executable for your platform from [GitHub Releases](https://github.com/Lebenoa/gredit-mcp/releases/latest):

- Windows: `gredit-mcp.exe`
- Linux: `gredit-mcp`

Or build from source with Rust and Cargo (Rust 2024 edition support):

```sh
cargo build --locked --release
```

The executable is in `target/release/` (`gredit-mcp.exe` on Windows).

## Quick start: stdio

Stdio is the default transport and is the simplest option for a local MCP client. Start the server with an existing workspace directory:

```powershell
cargo run --release -- C:\path\to\workspace
```

With a downloaded Windows executable:

```powershell
C:\path\to\gredit-mcp.exe C:\path\to\workspace
```

If the workspace argument is omitted, the server uses `GREDIT_WORKSPACE` when set, otherwise the process's current directory.

### MCP client configuration

For clients that accept the common `mcpServers` JSON format, configure a stdio server like this. Replace both paths with local absolute paths:

```json
{
  "mcpServers": {
    "gredit": {
      "command": "C:\\path\\to\\gredit-mcp.exe",
      "args": ["C:\\path\\to\\workspace"]
    }
  }
}
```

You can use Cargo instead of a downloaded executable by setting `command` to `cargo` and `args` to `run`, `--quiet`, `--manifest-path`, the path to this repository's `Cargo.toml`, `--`, and the workspace path. Client configuration locations vary; follow your MCP client's documentation.

## Tools

All file and directory arguments are relative to the active workspace. Absolute paths and `..` components are rejected, and resolved file-tool paths must remain inside the workspace. Tool results are structured JSON with output schemas advertised through MCP `tools/list`.

| Tool | Purpose | Common arguments |
| --- | --- | --- |
| `read` | Read a UTF-8 text file with 1-based line numbers. | `path`; optional `start_line`, `end_line`, `max_bytes` |
| `write` | Create or overwrite a UTF-8 text file. | `path`, `content`; optional `create_dirs` |
| `edit` | Replace exact text in a file. By default, the old text must match exactly once. | `path`, `old_string`, `new_string`; optional `replace_all` |
| `list` | List directory entries, optionally recursively. | optional `path`, `recursive`, `max_entries` |
| `grep` | Search text files with ripgrep's embedded search engine. | `query`; optional `path`, `file_suffix`, `max_results`, `case_insensitive` |
| `set_workspace` | Switch to another existing workspace after user approval. | absolute directory `path` |
| `exec` | Evaluate a Nushell command in the active workspace. | `command`; optional `working_dir`, `env`, `timeout_ms`, `max_output_bytes` |

`grep` respects standard ignore rules such as `.gitignore`, skips hidden files and symlinks, and returns matching paths, line numbers, and line text.

### Changing the workspace

`set_workspace` asks the connected MCP client to approve the exact canonical path before switching. The client must support MCP elicitation. If approval is declined, unavailable, or does not match the requested path, the current workspace is unchanged. Network sessions keep their workspace state separate.

## Transports

### Stdio

Stdio is the default. It runs one local MCP server process per client configuration and enables `exec` because the process is launched by the client directly. Only configure it for clients you trust.

### Streamable HTTP

```powershell
cargo run --release -- --http --addr 127.0.0.1:3000 C:\path\to\workspace
```

The MCP endpoint is `http://127.0.0.1:3000/mcp`. It supports JSON-RPC requests over `POST` and an SSE stream over `GET` with `Accept: text/event-stream`. The default address is `127.0.0.1:3000`.

### WebSocket

```powershell
cargo run --release -- --ws --addr 127.0.0.1:8080 C:\path\to\workspace
```

The default address is `127.0.0.1:8080`. Each WebSocket connection has its own MCP session, and each frame carries one JSON-RPC message.

`--addr` accepts loopback addresses only. HTTP and WebSocket are unauthenticated by default, and network `exec` is disabled by default.

### Network authentication and command execution

To require a shared bearer token for all HTTP requests or WebSocket upgrades, set `GREDIT_MCP_BEARER_TOKEN`. The MCP client must send `Authorization: Bearer <token>` on every network request. Configure a high-entropy secret outside the command line; for example, in PowerShell:

```powershell
$env:GREDIT_MCP_BEARER_TOKEN = "replace-with-a-long-random-secret"
cargo run --release -- --http C:\path\to\workspace
```

Authentication does **not** enable command execution. To expose `exec` over HTTP or WebSocket, explicitly add `--allow-remote-exec`:

```powershell
cargo run --release -- --http --allow-remote-exec C:\path\to\workspace
```

Without a bearer token, every client that can reach the listener can use the enabled tools, including `exec` when that flag is present. The server binds only to loopback and does not provide TLS; do not expose it to an untrusted network. Use a TLS-terminating authenticated proxy or tunnel for remote access, and do not send bearer tokens over plain HTTP on an untrusted network.

## Nushell execution

`exec` uses an embedded Nushell engine; a separate Nushell installation is not required. Nushell's `run-external` can launch ordinary programs. **Execution is not a sandbox:** commands and external programs run with the server process's operating-system permissions and may access resources outside the workspace. Treat stdio clients—and any network client granted `exec`—as fully trusted.

The `working_dir` argument is workspace-relative and defaults to the workspace root. Additional environment variables can be passed in `env`. The default timeout is 30 seconds (maximum 5 minutes); output is capped at 256 KiB by default and 4 MiB per stream at most.

For structured output from built-in Nushell commands, pipe to `to json`:

```nushell
ls | to json
```

Do not pipe external text output such as `git diff` to `to json`; that can truncate the output. External programs can still be run with `run-external` syntax.

Example tool arguments:

```json
{
  "command": "ls | where type == file | get name",
  "working_dir": "src",
  "timeout_ms": 10000
}
```

## Limits

| Tool | Default | Maximum |
| --- | --- | --- |
| `read` | 1 MiB per file | 8 MiB (`max_bytes`) |
| `list` | 100 entries | 1,000 (`max_entries`) |
| `grep` | 100 matching lines | 1,000 (`max_results`); 20,000 files and 512 MiB scanned |
| `exec` | 30-second timeout; 256 KiB output per stream | 5-minute timeout; 4 MiB output per stream |

## Development

```sh
cargo fmt --all -- --check
cargo clippy --locked --all-targets -- -D warnings
cargo test --locked --all-targets
```

Tagged `v*` pushes run the same checks, build Windows and Linux release binaries, and attach them to a GitHub Release.
