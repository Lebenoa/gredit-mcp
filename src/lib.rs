use std::{
    fs, io,
    path::{Component, Path, PathBuf},
    process::Stdio,
    sync::RwLock,
};

use tokio::{
    io::{AsyncRead, AsyncReadExt},
    process::Command,
    time::{Duration, timeout},
};

use rmcp::{
    ErrorData as McpError, handler::server::wrapper::Parameters, schemars::JsonSchema, tool,
    tool_router,
};
use rmcp::{RoleServer, service::RequestContext};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
struct WorkspaceApproval {
    #[schemars(description = "The exact absolute workspace path the user approved")]
    approved_path: String,
}

rmcp::elicit_safe!(WorkspaceApproval);

const DEFAULT_MAX_READ_BYTES: usize = 1_048_576;
const MAX_READ_BYTES: usize = 8 * 1_048_576;
const DEFAULT_MAX_RESULTS: usize = 100;
const MAX_RESULTS: usize = 1_000;
const DEFAULT_EXEC_TIMEOUT_MS: u64 = 30_000;
const MAX_EXEC_TIMEOUT_MS: u64 = 300_000;
const DEFAULT_EXEC_OUTPUT_BYTES: usize = 256 * 1_024;
const MAX_EXEC_OUTPUT_BYTES: usize = 4 * 1_048_576;

#[derive(Debug)]
pub struct FileSystemServer {
    root: RwLock<PathBuf>,
}

impl FileSystemServer {
    pub fn new(root: impl AsRef<Path>) -> io::Result<Self> {
        let root = fs::canonicalize(root)?;
        if !root.is_dir() {
            return Err(io::Error::new(
                io::ErrorKind::NotADirectory,
                format!("workspace root is not a directory: {}", root.display()),
            ));
        }
        Ok(Self {
            root: RwLock::new(root),
        })
    }

    pub fn root(&self) -> PathBuf {
        self.root
            .read()
            .expect("workspace root lock poisoned")
            .clone()
    }

    fn set_root(&self, root: impl AsRef<Path>) -> io::Result<()> {
        let root = fs::canonicalize(root)?;
        if !root.is_dir() {
            return Err(io::Error::new(
                io::ErrorKind::NotADirectory,
                format!("workspace root is not a directory: {}", root.display()),
            ));
        }
        *self.root.write().expect("workspace root lock poisoned") = root;
        Ok(())
    }

    fn resolve_existing(&self, input: &str) -> Result<PathBuf, String> {
        let relative = validate_relative_path(input)?;
        let root = self.root();
        let path = root.join(relative);
        let resolved = fs::canonicalize(&path)
            .map_err(|error| format!("cannot resolve '{}': {error}", input))?;
        self.ensure_inside_root(&root, &resolved)?;
        Ok(resolved)
    }

    fn resolve_for_create(&self, input: &str) -> Result<PathBuf, String> {
        let relative = validate_relative_path(input)?;
        let root = self.root();
        let path = root.join(relative);
        let mut existing = path.as_path();
        let mut missing = Vec::new();

        while !existing.exists() {
            let name = existing
                .file_name()
                .ok_or_else(|| format!("path has no file name: '{input}'"))?;
            missing.push(name.to_owned());
            existing = existing
                .parent()
                .ok_or_else(|| format!("path has no parent: '{input}'"))?;
        }

        let mut resolved = fs::canonicalize(existing)
            .map_err(|error| format!("cannot resolve parent of '{}': {error}", input))?;
        self.ensure_inside_root(&root, &resolved)?;
        for component in missing.iter().rev() {
            resolved.push(component);
        }
        Ok(resolved)
    }

    fn resolve_directory(&self, input: &str) -> Result<PathBuf, String> {
        let path = self.resolve_existing(input)?;
        if path.is_dir() {
            Ok(path)
        } else {
            Err(format!("'{}' is not a directory", input))
        }
    }

    fn ensure_inside_root(&self, root: &Path, path: &Path) -> Result<(), String> {
        if path == root || path.starts_with(root) {
            Ok(())
        } else {
            Err(format!(
                "path '{}' is outside the workspace root",
                path.display()
            ))
        }
    }

    fn display_path(&self, path: &Path) -> String {
        let root = self.root();
        path.strip_prefix(&root)
            .map(|relative| {
                let value = relative.to_string_lossy().replace('\\', "/");
                if value.is_empty() {
                    ".".to_owned()
                } else {
                    value
                }
            })
            .unwrap_or_else(|_| path.to_string_lossy().into_owned())
    }

    fn result_limit(
        value: Option<usize>,
        name: &str,
        default: usize,
        max: usize,
    ) -> Result<usize, String> {
        let value = value.unwrap_or(default);
        if value == 0 || value > max {
            Err(format!("{name} must be between 1 and {max}"))
        } else {
            Ok(value)
        }
    }
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct SetWorkspaceRequest {
    #[schemars(description = "Absolute directory path to use as the new workspace root")]
    pub path: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ReadRequest {
    #[schemars(description = "Workspace-relative file path")]
    pub path: String,
    #[schemars(description = "Optional 1-based first line to return")]
    pub start_line: Option<usize>,
    #[schemars(description = "Optional 1-based inclusive last line to return")]
    pub end_line: Option<usize>,
    #[schemars(description = "Maximum UTF-8 bytes to read")]
    pub max_bytes: Option<usize>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct WriteRequest {
    #[schemars(description = "Workspace-relative file path")]
    pub path: String,
    #[schemars(description = "Complete UTF-8 file contents")]
    pub content: String,
    #[schemars(description = "Create missing parent directories")]
    pub create_dirs: Option<bool>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct EditRequest {
    #[schemars(description = "Workspace-relative file path")]
    pub path: String,
    #[schemars(description = "Exact text to replace; it must not be empty")]
    pub old_string: String,
    #[schemars(description = "Replacement text")]
    pub new_string: String,
    #[schemars(description = "Replace every match instead of requiring exactly one")]
    pub replace_all: Option<bool>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ListRequest {
    #[schemars(description = "Workspace-relative directory path; defaults to the workspace root")]
    pub path: Option<String>,
    #[schemars(description = "Include nested entries")]
    pub recursive: Option<bool>,
    #[schemars(description = "Maximum entries to return")]
    pub max_entries: Option<usize>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct GrepRequest {
    #[schemars(description = "Regular expression matched against each text line")]
    pub query: String,
    #[schemars(
        description = "Workspace-relative file or directory; defaults to the workspace root"
    )]
    pub path: Option<String>,
    #[schemars(description = "Only search files whose names end with this suffix, such as '.rs'")]
    pub file_suffix: Option<String>,
    #[schemars(description = "Maximum matching lines to return")]
    pub max_results: Option<usize>,
    #[schemars(description = "Case-insensitive matching")]
    pub case_insensitive: Option<bool>,
}

/// Parameters for the `exec` tool.
///
/// The command is executed as a single argument to the selected shell, not
/// interpolated by this server. By default the invocation is `nu -c <command>`.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct ExecRequest {
    #[schemars(description = "Command string passed to the selected shell")]
    pub command: String,
    #[schemars(description = "Shell executable; defaults to 'nu' (examples: pwsh, bash, sh)")]
    pub shell: Option<String>,
    #[schemars(
        description = "Optional arguments inserted before the command; defaults to '-c' (pwsh uses '-Command')"
    )]
    pub shell_args: Option<Vec<String>>,
    #[schemars(
        description = "Workspace-relative working directory; defaults to the workspace root"
    )]
    pub working_dir: Option<String>,
    #[schemars(description = "Additional environment variables for the child process")]
    pub env: Option<std::collections::BTreeMap<String, String>>,
    #[schemars(description = "Execution timeout in milliseconds")]
    pub timeout_ms: Option<u64>,
    #[schemars(description = "Maximum bytes returned for each stdout and stderr stream")]
    pub max_output_bytes: Option<usize>,
}

#[tool_router(server_handler)]
impl FileSystemServer {
    #[tool(
        description = "Request user approval and switch the active workspace root to an absolute directory"
    )]
    pub async fn set_workspace(
        &self,
        context: RequestContext<RoleServer>,
        Parameters(request): Parameters<SetWorkspaceRequest>,
    ) -> Result<String, McpError> {
        let requested_input = Path::new(&request.path);
        if !requested_input.is_absolute() {
            return Err(tool_error("workspace path must be absolute"));
        }
        let requested = fs::canonicalize(requested_input)
            .map_err(|error| tool_error(format!("cannot resolve requested workspace: {error}")))?;
        if !requested.is_dir() {
            return Err(tool_error(format!(
                "workspace path is not a directory: {}",
                requested.display()
            )));
        }
        let requested = requested.to_string_lossy().into_owned();
        let Some(approval) = context
            .peer
            .elicit::<WorkspaceApproval>(format!(
                "Allow this server to switch its workspace to '{}'? This changes the directory used by read, write, edit, list, grep, and exec.",
                requested
            ))
            .await
            .map_err(|error| tool_error(format!("workspace approval failed: {error}")))?
        else {
            return Err(tool_error("workspace change was not approved"));
        };
        if approval.approved_path != requested {
            return Err(tool_error(
                "workspace approval did not match the requested path",
            ));
        }
        self.set_root(&requested)
            .map_err(|error| tool_error(format!("cannot switch workspace: {error}")))?;
        Ok(format!("workspace changed to {}", self.root().display()))
    }

    #[tool(description = "Read a UTF-8 text file with stable 1-based line numbers")]
    pub fn read(&self, Parameters(request): Parameters<ReadRequest>) -> Result<String, McpError> {
        let path = self.resolve_existing(&request.path).map_err(tool_error)?;
        if !path.is_file() {
            return Err(tool_error(format!("'{}' is not a file", request.path)));
        }

        let max_bytes = request.max_bytes.unwrap_or(DEFAULT_MAX_READ_BYTES);
        if max_bytes == 0 || max_bytes > MAX_READ_BYTES {
            return Err(tool_error(format!(
                "max_bytes must be between 1 and {MAX_READ_BYTES}"
            )));
        }
        let metadata = fs::metadata(&path).map_err(io_tool_error)?;
        if metadata.len() > max_bytes as u64 {
            return Err(tool_error(format!(
                "file is {} bytes; max_bytes is {max_bytes}",
                metadata.len()
            )));
        }
        let contents = fs::read_to_string(&path).map_err(io_tool_error)?;
        let lines: Vec<&str> = contents.lines().collect();
        let start = request.start_line.unwrap_or(1);
        let end = request.end_line.unwrap_or(lines.len().max(1));
        if start == 0 || end == 0 || start > end {
            return Err(tool_error(
                "start_line and end_line must be positive, and start_line <= end_line",
            ));
        }

        let selected = lines
            .iter()
            .enumerate()
            .filter_map(|(index, line)| {
                let line_number = index + 1;
                (line_number >= start && line_number <= end)
                    .then(|| format!("{line_number:>6} | {line}"))
            })
            .collect::<Vec<_>>();
        let range = if selected.is_empty() {
            format!("lines {start}..={end} (file has {} lines)", lines.len())
        } else {
            selected.join("\n")
        };
        Ok(format!("{}\n{range}", self.display_path(&path)))
    }

    #[tool(description = "Create or overwrite a UTF-8 text file inside the workspace")]
    pub fn write(&self, Parameters(request): Parameters<WriteRequest>) -> Result<String, McpError> {
        let path = self.resolve_for_create(&request.path).map_err(tool_error)?;
        if path.exists() && path.is_dir() {
            return Err(tool_error(format!("'{}' is a directory", request.path)));
        }
        if request.create_dirs.unwrap_or(false) {
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent).map_err(io_tool_error)?;
            }
        } else if path.parent().is_some_and(|parent| !parent.is_dir()) {
            return Err(tool_error(format!(
                "parent directory does not exist for '{}' (set create_dirs=true)",
                request.path
            )));
        }
        fs::write(&path, request.content.as_bytes()).map_err(io_tool_error)?;
        Ok(format!("wrote {}", self.display_path(&path)))
    }

    #[tool(description = "Apply an exact text edit to a UTF-8 file")]
    pub fn edit(&self, Parameters(request): Parameters<EditRequest>) -> Result<String, McpError> {
        if request.old_string.is_empty() {
            return Err(tool_error("old_string must not be empty"));
        }
        let path = self.resolve_existing(&request.path).map_err(tool_error)?;
        if !path.is_file() {
            return Err(tool_error(format!("'{}' is not a file", request.path)));
        }
        let contents = fs::read_to_string(&path).map_err(io_tool_error)?;
        let matches = contents.match_indices(&request.old_string).count();
        if matches == 0 {
            return Err(tool_error("old_string was not found"));
        }
        if matches > 1 && !request.replace_all.unwrap_or(false) {
            return Err(tool_error(format!(
                "old_string matched {matches} times; set replace_all=true to replace all"
            )));
        }
        let updated = if request.replace_all.unwrap_or(false) {
            contents.replace(&request.old_string, &request.new_string)
        } else {
            contents.replacen(&request.old_string, &request.new_string, 1)
        };
        fs::write(&path, updated.as_bytes()).map_err(io_tool_error)?;
        Ok(format!(
            "edited {} ({} replacement{})",
            self.display_path(&path),
            if request.replace_all.unwrap_or(false) {
                matches
            } else {
                1
            },
            if matches == 1 { "" } else { "s" }
        ))
    }

    #[tool(
        description = "Execute a command in the workspace using a selectable shell; defaults to Nushell"
    )]
    pub async fn exec(
        &self,
        Parameters(request): Parameters<ExecRequest>,
    ) -> Result<String, McpError> {
        if request.command.trim().is_empty() {
            return Err(tool_error("command must not be empty"));
        }
        let timeout_ms = request.timeout_ms.unwrap_or(DEFAULT_EXEC_TIMEOUT_MS);
        if timeout_ms == 0 || timeout_ms > MAX_EXEC_TIMEOUT_MS {
            return Err(tool_error(format!(
                "timeout_ms must be between 1 and {MAX_EXEC_TIMEOUT_MS}"
            )));
        }
        let max_output_bytes = request
            .max_output_bytes
            .unwrap_or(DEFAULT_EXEC_OUTPUT_BYTES);
        if max_output_bytes == 0 || max_output_bytes > MAX_EXEC_OUTPUT_BYTES {
            return Err(tool_error(format!(
                "max_output_bytes must be between 1 and {MAX_EXEC_OUTPUT_BYTES}"
            )));
        }

        let shell = request.shell.unwrap_or_else(|| "nu".to_owned());
        if shell.trim().is_empty() || shell.contains('\0') {
            return Err(tool_error("shell must not be empty or contain NUL bytes"));
        }
        let working_dir = self
            .resolve_directory(request.working_dir.as_deref().unwrap_or("."))
            .map_err(tool_error)?;
        let args = shell_arguments(&shell, request.shell_args.as_deref(), &request.command);

        let mut process = Command::new(&shell);
        process
            .args(args)
            .current_dir(&working_dir)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        if let Some(environment) = request.env.as_ref() {
            process.envs(environment);
        }
        let mut child = process
            .spawn()
            .map_err(|error| tool_error(format!("failed to start shell '{shell}': {error}")))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| tool_error("failed to capture command stdout"))?;
        let stderr = child
            .stderr
            .take()
            .ok_or_else(|| tool_error("failed to capture command stderr"))?;

        let execution = timeout(Duration::from_millis(timeout_ms), async {
            let stdout = read_capped(stdout, max_output_bytes);
            let stderr = read_capped(stderr, max_output_bytes);
            let (stdout, stderr, status) = tokio::join!(stdout, stderr, child.wait());
            Ok::<_, io::Error>((stdout?, stderr?, status?))
        })
        .await;

        let (stdout, stderr, status) = match execution {
            Ok(Ok(result)) => result,
            Ok(Err(error)) => return Err(io_tool_error(error)),
            Err(_) => {
                child.kill().await.map_err(io_tool_error)?;
                let _ = child.wait().await;
                return Err(tool_error(format!(
                    "command timed out after {timeout_ms} ms"
                )));
            }
        };

        let (stdout, stdout_truncated) = stdout;
        let (stderr, stderr_truncated) = stderr;
        let mut output = String::new();
        output.push_str(&format!(
            "shell: {shell}\nworking_dir: {}\nexit_code: {}\n",
            self.display_path(&working_dir),
            status
                .code()
                .map_or_else(|| "signal/unknown".to_owned(), |code| code.to_string())
        ));
        append_output_section(&mut output, "stdout", &stdout);
        append_output_section(&mut output, "stderr", &stderr);
        if stdout_truncated || stderr_truncated {
            output.push_str(&format!(
                "output_truncated: true (per-stream limit: {max_output_bytes} bytes)\n"
            ));
        }
        Ok(output)
    }

    #[tool(description = "List workspace files and directories")]
    pub fn list(&self, Parameters(request): Parameters<ListRequest>) -> Result<String, McpError> {
        let path = self
            .resolve_directory(request.path.as_deref().unwrap_or("."))
            .map_err(tool_error)?;
        let max_entries = Self::result_limit(
            request.max_entries,
            "max_entries",
            DEFAULT_MAX_RESULTS,
            MAX_RESULTS,
        )
        .map_err(tool_error)?;
        let mut entries = Vec::new();
        collect_entries(
            &self.root(),
            &path,
            request.recursive.unwrap_or(false),
            max_entries,
            &mut entries,
        )
        .map_err(io_tool_error)?;
        entries.sort_unstable();
        let output = entries
            .into_iter()
            .take(max_entries)
            .collect::<Vec<_>>()
            .join("\n");
        Ok(if output.is_empty() {
            format!("{}\n(empty)", self.display_path(&path))
        } else {
            format!("{}\n{output}", self.display_path(&path))
        })
    }

    #[tool(description = "Search text files with a regular expression")]
    pub fn grep(&self, Parameters(request): Parameters<GrepRequest>) -> Result<String, McpError> {
        if request.query.is_empty() {
            return Err(tool_error("query must not be empty"));
        }
        let regex = regex::RegexBuilder::new(&request.query)
            .case_insensitive(request.case_insensitive.unwrap_or(false))
            .build()
            .map_err(|error| tool_error(format!("invalid regular expression: {error}")))?;
        let max_results = Self::result_limit(
            request.max_results,
            "max_results",
            DEFAULT_MAX_RESULTS,
            MAX_RESULTS,
        )
        .map_err(tool_error)?;
        let path = self
            .resolve_existing(request.path.as_deref().unwrap_or("."))
            .map_err(tool_error)?;
        let suffix = request.file_suffix.as_deref();
        let mut matches = Vec::new();
        search_path(
            &self.root(),
            &path,
            suffix,
            &regex,
            max_results,
            &mut matches,
        )
        .map_err(io_tool_error)?;
        matches.sort_unstable();
        let output = matches
            .into_iter()
            .take(max_results)
            .collect::<Vec<_>>()
            .join("\n");
        Ok(if output.is_empty() {
            "(no matches)".to_owned()
        } else {
            output
        })
    }
}

fn default_shell_flag(shell: &str) -> String {
    let shell_name = Path::new(shell)
        .file_stem()
        .and_then(|name| name.to_str())
        .unwrap_or(shell)
        .to_ascii_lowercase();
    if matches!(shell_name.as_str(), "pwsh" | "powershell") {
        "-Command".to_owned()
    } else if shell_name == "cmd" {
        "/C".to_owned()
    } else {
        "-c".to_owned()
    }
}

fn shell_arguments(shell: &str, shell_args: Option<&[String]>, command: &str) -> Vec<String> {
    let mut args = shell_args.map_or_else(Vec::new, ToOwned::to_owned);
    if args.is_empty() {
        args.push(default_shell_flag(shell));
    }
    args.push(command.to_owned());
    args
}

async fn read_capped<R>(mut reader: R, limit: usize) -> io::Result<(String, bool)>
where
    R: AsyncRead + Unpin,
{
    let mut bytes = Vec::new();
    let mut buffer = [0_u8; 8 * 1_024];
    let mut truncated = false;
    loop {
        let read = reader.read(&mut buffer).await?;
        if read == 0 {
            break;
        }
        let remaining = limit.saturating_sub(bytes.len());
        if read > remaining {
            bytes.extend_from_slice(&buffer[..remaining]);
            truncated = true;
            break;
        }
        bytes.extend_from_slice(&buffer[..read]);
    }
    let text = String::from_utf8_lossy(&bytes).into_owned();
    Ok((text, truncated))
}

fn append_output_section(output: &mut String, name: &str, content: &str) {
    output.push_str(&format!("{name}:\n"));
    if content.is_empty() {
        output.push_str("(empty)\n");
    } else {
        output.push_str(content);
        if !content.ends_with('\n') {
            output.push('\n');
        }
    }
}

fn validate_relative_path(input: &str) -> Result<&Path, String> {
    if input.trim().is_empty() {
        return Err("path must not be empty".to_owned());
    }
    let path = Path::new(input);
    if path.is_absolute() {
        return Err("path must be workspace-relative".to_owned());
    }
    if path
        .components()
        .any(|component| matches!(component, Component::ParentDir | Component::Prefix(_)))
    {
        return Err("path must not contain '..' or a platform prefix".to_owned());
    }
    Ok(path)
}

fn collect_entries(
    root: &Path,
    directory: &Path,
    recursive: bool,
    max_entries: usize,
    entries: &mut Vec<String>,
) -> io::Result<()> {
    let mut children = fs::read_dir(directory)?.collect::<Result<Vec<_>, _>>()?;
    children.sort_unstable_by_key(|entry| entry.file_name());
    for entry in children {
        if entries.len() >= max_entries {
            break;
        }
        let path = entry.path();
        let label = if entry.file_type()?.is_dir() {
            format!("[dir]  {}", display_relative(root, &path))
        } else {
            format!("[file] {}", display_relative(root, &path))
        };
        entries.push(label);
        if recursive && entry.file_type()?.is_dir() {
            collect_entries(root, &path, recursive, max_entries, entries)?;
        }
    }
    Ok(())
}

fn search_path(
    root: &Path,
    path: &Path,
    suffix: Option<&str>,
    regex: &regex::Regex,
    max_results: usize,
    matches: &mut Vec<String>,
) -> io::Result<()> {
    if matches.len() >= max_results {
        return Ok(());
    }
    let metadata = fs::symlink_metadata(path)?;
    if metadata.is_dir() {
        let mut children = fs::read_dir(path)?.collect::<Result<Vec<_>, _>>()?;
        children.sort_unstable_by_key(|entry| entry.file_name());
        for entry in children {
            search_path(root, &entry.path(), suffix, regex, max_results, matches)?;
            if matches.len() >= max_results {
                break;
            }
        }
        return Ok(());
    }
    if !metadata.is_file() || suffix.is_some_and(|suffix| !path.to_string_lossy().ends_with(suffix))
    {
        return Ok(());
    }
    let contents = match fs::read_to_string(path) {
        Ok(contents) => contents,
        Err(error) if error.kind() == io::ErrorKind::InvalidData => return Ok(()),
        Err(error) => return Err(error),
    };
    for (index, line) in contents.lines().enumerate() {
        if regex.is_match(line) {
            matches.push(format!(
                "{}:{}: {}",
                display_relative(root, path),
                index + 1,
                line
            ));
            if matches.len() >= max_results {
                break;
            }
        }
    }
    Ok(())
}

fn display_relative(root: &Path, path: &Path) -> String {
    path.strip_prefix(root)
        .map(|relative| {
            let value = relative.to_string_lossy().replace('\\', "/");
            if value.is_empty() {
                ".".to_owned()
            } else {
                value
            }
        })
        .unwrap_or_else(|_| path.to_string_lossy().into_owned())
}

fn tool_error(message: impl Into<String>) -> McpError {
    McpError::internal_error(message.into(), None)
}

fn io_tool_error(error: io::Error) -> McpError {
    tool_error(error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn server() -> (tempfile::TempDir, FileSystemServer) {
        let directory = tempdir().expect("temp directory");
        let server = FileSystemServer::new(directory.path()).expect("server");
        (directory, server)
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

        let error = server
            .edit(Parameters(EditRequest {
                path: "example.txt".to_owned(),
                old_string: "one".to_owned(),
                new_string: "two".to_owned(),
                replace_all: None,
            }))
            .expect_err("ambiguous edit should fail");
        assert!(error.message.contains("matched 2 times"));

        let result = server
            .edit(Parameters(EditRequest {
                path: "example.txt".to_owned(),
                old_string: "one".to_owned(),
                new_string: "two".to_owned(),
                replace_all: Some(true),
            }))
            .expect("replace all");
        assert!(result.contains("2 replacements"));
        assert_eq!(
            fs::read_to_string(file).expect("read fixture"),
            "two\ntwo\n"
        );
    }

    #[test]
    fn workspace_requires_an_absolute_path() {
        assert!(!Path::new("relative/path").is_absolute());
    }

    #[test]
    fn workspace_can_switch_after_validation() {
        let first = tempdir().expect("first temp directory");
        let second = tempdir().expect("second temp directory");
        let server = FileSystemServer::new(first.path()).expect("server");
        server.set_root(second.path()).expect("switch root");
        assert_eq!(
            server.root(),
            fs::canonicalize(second.path()).expect("canonical root")
        );
    }

    #[test]
    fn shell_arguments_default_for_common_shells() {
        assert_eq!(default_shell_flag("nu"), "-c");
        assert_eq!(default_shell_flag("pwsh"), "-Command");
        assert_eq!(default_shell_flag("powershell.exe"), "-Command");
        assert_eq!(default_shell_flag("cmd.exe"), "/C");
        assert_eq!(default_shell_flag("bash"), "-c");
        assert_eq!(
            shell_arguments("nu", None, "echo hi"),
            vec!["-c", "echo hi"]
        );
        assert_eq!(
            shell_arguments("pwsh", None, "Write-Output hi"),
            vec!["-Command", "Write-Output hi"]
        );
        assert_eq!(
            shell_arguments(
                "bash",
                Some(&["--noprofile".to_owned(), "-c".to_owned()]),
                "echo hi"
            ),
            vec!["--noprofile", "-c", "echo hi"]
        );
    }

    #[tokio::test]
    async fn exec_runs_in_workspace_with_requested_shell() {
        let (_directory, server) = server();
        let result = server
            .exec(Parameters(ExecRequest {
                command: "Write-Output hello".to_owned(),
                shell: Some("pwsh".to_owned()),
                shell_args: None,
                working_dir: None,
                env: None,
                timeout_ms: Some(5_000),
                max_output_bytes: Some(1_024),
            }))
            .await
            .expect("exec");
        assert!(result.contains("exit_code: 0"));
        assert!(result.contains("hello"));
    }

    #[tokio::test]
    async fn exec_rejects_outside_working_directory() {
        let (_directory, server) = server();
        let error = server
            .exec(Parameters(ExecRequest {
                command: "Write-Output hello".to_owned(),
                shell: Some("pwsh".to_owned()),
                shell_args: None,
                working_dir: Some("../".to_owned()),
                env: None,
                timeout_ms: None,
                max_output_bytes: None,
            }))
            .await
            .expect_err("outside working directory should fail");
        assert!(error.message.contains(".."));
    }

    #[test]
    fn write_read_and_grep_work_inside_root() {
        let (_directory, server) = server();
        server
            .write(Parameters(WriteRequest {
                path: "src/example.rs".to_owned(),
                content: "fn main() {}\n".to_owned(),
                create_dirs: Some(true),
            }))
            .expect("write file");
        let read = server
            .read(Parameters(ReadRequest {
                path: "src/example.rs".to_owned(),
                start_line: None,
                end_line: None,
                max_bytes: None,
            }))
            .expect("read file");
        assert!(read.contains("1 | fn main() {}"));

        let grep = server
            .grep(Parameters(GrepRequest {
                query: "fn main".to_owned(),
                path: None,
                file_suffix: Some(".rs".to_owned()),
                max_results: None,
                case_insensitive: None,
            }))
            .expect("grep");
        assert!(grep.contains("src/example.rs:1: fn main() {}"));
    }

    #[test]
    fn list_is_deterministic_and_relative() {
        let (_directory, server) = server();
        fs::create_dir(server.root().join("nested")).expect("directory");
        fs::write(server.root().join("b.txt"), "b").expect("file");
        fs::write(server.root().join("nested/a.txt"), "a").expect("file");
        let listing = server
            .list(Parameters(ListRequest {
                path: None,
                recursive: Some(true),
                max_entries: None,
            }))
            .expect("list");
        assert!(listing.contains("[dir]  nested"));
        assert!(listing.contains("[file] nested/a.txt"));
        assert!(listing.contains("[file] b.txt"));
    }
}
