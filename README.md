# gredit-mcp

A Rust MCP server for workspace-aware agent tools.

## Run

```powershell
cargo run -- C:\path\to\workspace
```

The workspace root can also be supplied with `GREDIT_WORKSPACE`. All
filesystem paths and command working directories remain inside the active
root. To use another directory, call `set_workspace` with an absolute path;
the MCP client must support elicitation and the user must approve the exact
path before the server switches roots.

## Network transports

Besides stdio (the default), the server exposes two network transports:

```powershell
# Streamable HTTP + SSE
cargo run -- --http --addr 127.0.0.1:3000 C:\path\to\workspace

# WebSocket
cargo run -- --ws --addr 127.0.0.1:8080 C:\path\to\workspace
```

- `--http` serves the MCP protocol at `/mcp` over Streamable HTTP
  (`POST /mcp` for JSON-RPC) with Server-Sent Events (`GET /mcp` with
  `Accept: text/event-stream` for the streaming response channel). Each
  session is isolated and rooted at the workspace path.
- `--ws` serves one MCP session per WebSocket connection; every JSON-RPC
  message travels in a WebSocket text frame.
- `--addr` defaults to `127.0.0.1:3000` for HTTP and `127.0.0.1:8080` for
  WebSocket.

## Tools

- `read` — read a UTF-8 text file with line numbers.
- `write` — create or overwrite a UTF-8 text file.
- `edit` — apply an exact text replacement.
- `list` — list files and directories.
- `grep` — search text files with a regular expression.
- `set_workspace` — request explicit user approval before switching the active workspace directory.
- `exec` — evaluate Nushell commands in a workspace directory using the embedded engine.

### `exec`

`exec` always evaluates the command with an embedded Nushell engine. No shell
executable, shell flags, or external Nushell installation are required. For
structured output from built-in Nushell commands, prefer piping to `to json`,
for example `ls | to json`. Do not add `to json` to external text commands
such as `git diff`, because it can truncate their output. Raw executables can
still be invoked with Nushell's `run-external` syntax. `working_dir` is
workspace-relative. Optional environment variables, a 30-second default
timeout (five-minute maximum), and bounded output capture are supported.

Example request shape:

```json
{
  "command": "ls | where type == file | get name",
  "working_dir": "src",
  "timeout_ms": 10000
}
```

## Release build

The release profile is optimized aggressively with fat LTO, one codegen unit,
abort-on-panic, and stripped symbols. Build the Windows x86_64 executable with:

```powershell
cargo build --locked --release
```

The artifact is written to `target\release\gredit-mcp.exe`.
