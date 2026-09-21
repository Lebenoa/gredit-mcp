use std::{
    io,
    path::{Component, Path},
};

use rmcp::ErrorData as McpError;

pub(crate) const DEFAULT_MAX_READ_BYTES: usize = 1_048_576;
pub(crate) const MAX_READ_BYTES: usize = 8 * 1_048_576;
pub(crate) const DEFAULT_MAX_RESULTS: usize = 100;
pub(crate) const MAX_RESULTS: usize = 1_000;
pub(crate) const DEFAULT_EXEC_TIMEOUT_MS: u64 = 30_000;
pub(crate) const MAX_EXEC_TIMEOUT_MS: u64 = 300_000;
pub(crate) const DEFAULT_EXEC_OUTPUT_BYTES: usize = 256 * 1_024;
pub(crate) const MAX_EXEC_OUTPUT_BYTES: usize = 4 * 1_048_576;

pub(crate) fn validate_relative_path(input: &str) -> Result<&Path, String> {
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

pub(crate) fn result_limit(
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

pub(crate) fn tool_error(message: impl Into<String>) -> McpError {
    McpError::internal_error(message.into(), None)
}

pub(crate) fn io_tool_error(error: io::Error) -> McpError {
    tool_error(error.to_string())
}
