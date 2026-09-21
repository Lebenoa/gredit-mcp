# gredit-mcp

A Rust MCP server for workspace-aware agent tools.

## Run

```powershell
cargo run -- C:\path\to\workspace
```

The workspace root can also be supplied with `GREDIT_WORKSPACE`. All
filesystem paths and command working directories must remain inside that root.

## Tools

- `read` — read a UTF-8 text file with line numbers.
- `write` — create or overwrite a UTF-8 text file.
- `edit` — apply an exact text replacement.
- `list` — list files and directories.
- `grep` — search text files with a regular expression.
- `exec` — execute a command in a workspace directory.

### `exec`

`exec` defaults to Nushell:

```text
nu -c '<command>'
```

Set `shell` to use another executable. The server chooses the command flag
for common shells:

- `nu`, `bash`, `sh`, and similar: `-c`
- `pwsh` and `powershell`: `-Command`
- `cmd`: `/C`

For a shell with a different convention, pass `shell_args`; the command is
appended as the final argument. `working_dir` is workspace-relative. Optional
environment variables, a 30-second default timeout (five-minute maximum),
and bounded stdout/stderr capture are supported.

Example request shape:

```json
{
  "command": "Get-ChildItem",
  "shell": "pwsh",
  "working_dir": "src",
  "timeout_ms": 10000
}
```
