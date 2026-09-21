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
executable, shell flags, or external Nushell installation are required. Raw
executables can still be invoked with Nushell's `run-external` syntax.
`working_dir` is workspace-relative. Optional environment variables, a
30-second default timeout (five-minute maximum), and bounded output capture
are supported.

Example request shape:

```json
{
  "command": "ls | where type == file | get name",
  "working_dir": "src",
  "timeout_ms": 10000
}
```
