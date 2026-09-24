# gredit-mcp

`gredit-mcp` is a Model Context Protocol (MCP) server that gives an MCP client workspace-scoped file tools and, in trusted stdio mode, an embedded Nushell command runner. It supports stdio, Streamable HTTP with SSE, and WebSocket transports.

## Requirements

- Rust and Cargo with support for the Rust 2024 edition.
- An MCP client that can connect using one of the transports below.

## Build and run

Run the server over stdio, using the given directory as its workspace:

```powershell
cargo run --release -- C:\path\to\workspace
```

If no directory is provided, `gredit-mcp` uses `GREDIT_WORKSPACE`, if set, or the current directory. The workspace must be an existing directory.

To build the release executable without starting it:

```powershell
cargo build --locked --release
```

The executable is written to `target\release\gredit-mcp.exe` on Windows (or `target/release/gredit-mcp` on Unix-like systems).

### Configure an MCP client for stdio

For clients that use the `mcpServers` JSON configuration format, add an entry like this and replace both paths with local absolute paths:

```json
{
  "mcpServers": {
    "gredit": {
      "command": "cargo",
      "args": [
        "run", "--quiet",
        "--manifest-path", "C:\\path\\to\\gredit-mcp\\Cargo.toml",
        "--",
        "C:\\path\\to\\workspace"
      ]
    }
  }
}
```

You can use the built executable instead of Cargo by setting `command` to its full path and `args` to the workspace path. Client configuration locations and formats vary; use your MCP client's documentation.

## Transports

Stdio is the default transport. Network transports are opt-in:

```powershell
# Streamable HTTP with SSE at http://127.0.0.1:3000/mcp
cargo run --release -- --http --addr 127.0.0.1:3000 C:\path\to\workspace

# WebSocket at ws://127.0.0.1:8080
cargo run --release -- --ws --addr 127.0.0.1:8080 C:\path\to\workspace
```

`--addr` is optional: HTTP defaults to `127.0.0.1:3000`, and WebSocket defaults to `127.0.0.1:8080`. HTTP exposes the MCP Streamable HTTP endpoint at `/mcp` (JSON-RPC via `POST`, streaming via `GET` with `Accept: text/event-stream`). WebSocket creates one MCP session per connection, with each WebSocket frame carrying a JSON-RPC message.

Network listeners are restricted to loopback and do not enable `exec`. If you need remote access, put an authenticated reverse proxy in front of the server; do not expose it directly to an untrusted network.

## Available tools

All filesystem paths are relative to the active workspace. Absolute paths and paths containing `..` are rejected, and resolved paths must remain inside the workspace.

| Tool | What it does |
| --- | --- |
| `read` | Read a UTF-8 text file, optionally selecting a 1-based line range. |
| `write` | Create or overwrite a UTF-8 text file; optionally create missing parent directories. |
| `edit` | Replace an exact, non-empty text string. By default it must match exactly once; set `replace_all` to replace every match. |
| `list` | List a directory's files and subdirectories, optionally recursively. |
| `grep` | Search text files with a regular expression; supports optional path, file suffix, result limit, and case-insensitive matching. |
| `set_workspace` | Request approval to switch to another existing absolute directory. |
| `exec` | Evaluate a Nushell command in the workspace. Available only over stdio. |

Tool results are structured JSON, and each tool advertises an output schema through MCP `tools/list`.

### Workspace approval

`set_workspace` asks the connected MCP client to elicit approval for the exact canonical directory path before changing the active root. The client must support MCP elicitation and the user must approve the exact requested path. A rejected, unavailable, or mismatched approval leaves the workspace unchanged. Network sessions manage their workspace independently.

### Nushell execution and trust

In stdio mode, `exec` evaluates commands using the embedded Nushell engine; it does not require a separately installed Nushell executable. Nushell's `run-external` syntax can invoke external programs, so treat stdio access as trusted command execution and connect only clients you trust. Network transports disable this tool.

The `working_dir` argument is workspace-relative and defaults to the workspace root. Optional environment variables can be supplied with `env`. Execution has a 30-second default timeout (5-minute maximum) and bounded output capture (256 KiB by default, up to 4 MiB per stream).

For structured results from built-in Nushell commands, pipe to `to json`, for example:

```nushell
ls | to json
```

Do not pipe external text output such as `git diff` to `to json`; it can truncate the output. External programs can still be run directly with Nushell's `run-external` syntax.

Example `exec` tool arguments:

```json
{
  "command": "ls | where type == file | get name",
  "working_dir": "src",
  "timeout_ms": 10000
}
```

## Limits and defaults

- `read`: 1 MiB maximum file size by default; `max_bytes` can be set up to 8 MiB.
- `list`: up to 100 entries by default; `max_entries` can be set up to 1,000.
- `grep`: up to 100 matching lines by default; `max_results` can be set up to 1,000. A scan is also capped at 20,000 files and 512 MiB.
- `exec`: 30-second default timeout, at most 5 minutes; output capture defaults to 256 KiB and is capped at 4 MiB per stream.

## Development

Run the test suite with:

```powershell
cargo test
```
