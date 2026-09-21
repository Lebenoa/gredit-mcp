use std::sync::{Arc, OnceLock};

use rmcp::{ErrorData as McpError, Json, handler::server::wrapper::Parameters, tool, tool_router};
use tokio::{
    sync::Semaphore,
    time::{Duration, timeout},
};

const MAX_CONCURRENT_EXECUTIONS: usize = 4;
static EXEC_SEMAPHORE: OnceLock<Arc<Semaphore>> = OnceLock::new();

use crate::{
    nu_engine,
    results::ExecOutput,
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
        description = "Evaluate a Nushell command with the embedded engine. For structured output from built-in Nushell commands, prefer piping to 'to json' (for example, 'ls | to json'); do not use it for external text commands such as 'git diff', because it can truncate their output."
    )]
    pub async fn exec(
        &self,
        Parameters(request): Parameters<ExecRequest>,
    ) -> Result<Json<ExecOutput>, McpError> {
        if !self.allow_exec {
            return Err(tool_error(
                "exec is disabled for this server; use trusted stdio mode to enable command execution",
            ));
        }
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

        let semaphore = EXEC_SEMAPHORE
            .get_or_init(|| Arc::new(Semaphore::new(MAX_CONCURRENT_EXECUTIONS)))
            .clone();
        let permit = semaphore
            .acquire_owned()
            .await
            .map_err(|_| tool_error("execution capacity is unavailable"))?;

        let working_dir = self
            .resolve_directory(request.working_dir.as_deref().unwrap_or("."))
            .map_err(tool_error)?;
        let command = request.command;
        let env = request.env;
        let display_working_dir = self.display_path(&working_dir);
        let evaluation = timeout(
            Duration::from_millis(timeout_ms),
            tokio::task::spawn_blocking(move || {
                let _permit = permit;
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
        Ok(Json(ExecOutput {
            engine: "embedded-nushell".to_owned(),
            working_dir: display_working_dir,
            exit_code: 0,
            stdout: output,
            output_truncated: truncated,
        }))
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
