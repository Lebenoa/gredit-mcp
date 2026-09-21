use std::{io, path::Path, process::Stdio};

use rmcp::{ErrorData as McpError, handler::server::wrapper::Parameters, tool, tool_router};
use tokio::{
    io::{AsyncRead, AsyncReadExt},
    process::Command,
    time::{Duration, timeout},
};

use crate::{
    shared::{
        DEFAULT_EXEC_OUTPUT_BYTES, DEFAULT_EXEC_TIMEOUT_MS, MAX_EXEC_OUTPUT_BYTES,
        MAX_EXEC_TIMEOUT_MS, io_tool_error, tool_error,
    },
    types::ExecRequest,
    workspace::FileSystemServer,
};

#[tool_router(router = exec_router, vis = "pub(crate)")]
impl FileSystemServer {
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

#[cfg(test)]
mod tests {
    use super::*;

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
}
