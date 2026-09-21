use rmcp::{ErrorData as McpError, handler::server::wrapper::Parameters, tool, tool_router};
use tokio::time::{Duration, timeout};

use crate::{
    nu_engine,
    shared::{
        DEFAULT_EXEC_OUTPUT_BYTES, DEFAULT_EXEC_TIMEOUT_MS, MAX_EXEC_OUTPUT_BYTES,
        MAX_EXEC_TIMEOUT_MS, tool_error,
    },
    types::ExecRequest,
    workspace::FileSystemServer,
};

#[tool_router(router = exec_router, vis = "pub(crate)")]
impl FileSystemServer {
    #[tool(
        description = "Evaluate a Nushell command in the workspace using the embedded Nushell engine"
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

        let working_dir = self
            .resolve_directory(request.working_dir.as_deref().unwrap_or("."))
            .map_err(tool_error)?;
        let command = request.command;
        let env = request.env;
        let display_working_dir = self.display_path(&working_dir);
        let evaluation = timeout(
            Duration::from_millis(timeout_ms),
            tokio::task::spawn_blocking(move || {
                nu_engine::evaluate(&command, &working_dir, env.as_ref())
            }),
        )
        .await;
        let output = match evaluation {
            Ok(Ok(Ok(output))) => output,
            Ok(Ok(Err(error))) => {
                return Err(tool_error(format!("Nushell evaluation failed: {error}")));
            }
            Ok(Err(error)) => {
                return Err(tool_error(format!("embedded Nushell task failed: {error}")));
            }
            Err(_) => {
                return Err(tool_error(format!(
                    "command timed out after {timeout_ms} ms"
                )));
            }
        };

        let (output, truncated) = cap_output(&output, max_output_bytes);
        let mut response = format!(
            "engine: embedded-nushell\nworking_dir: {display_working_dir}\nexit_code: 0\nstdout:\n"
        );
        if output.is_empty() {
            response.push_str("(empty)\n");
        } else {
            response.push_str(&output);
            if !output.ends_with('\n') {
                response.push('\n');
            }
        }
        if truncated {
            response.push_str(&format!(
                "output_truncated: true (limit: {max_output_bytes} bytes)\n"
            ));
        }
        Ok(response)
    }
}

fn cap_output(output: &str, limit: usize) -> (String, bool) {
    if output.len() <= limit {
        return (output.to_owned(), false);
    }
    let mut end = limit;
    while !output.is_char_boundary(end) {
        end -= 1;
    }
    (output[..end].to_owned(), true)
}

#[cfg(test)]
mod tests {
    use super::cap_output;

    #[test]
    fn caps_output_without_splitting_utf8() {
        let (output, truncated) = cap_output("😀hello", 2);
        assert_eq!(output, "");
        assert!(truncated);
    }
}
