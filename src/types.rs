use std::collections::BTreeMap;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
pub(crate) struct WorkspaceApproval {
    #[schemars(description = "The exact absolute workspace path the user approved")]
    pub(crate) approved_path: String,
}

rmcp::elicit_safe!(WorkspaceApproval);

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
/// The command is evaluated by the embedded Nushell engine.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct ExecRequest {
    #[schemars(description = "Nushell command or script to evaluate")]
    pub command: String,
    #[schemars(
        description = "Workspace-relative working directory; defaults to the workspace root"
    )]
    pub working_dir: Option<String>,
    #[schemars(description = "Additional environment variables for the command")]
    pub env: Option<BTreeMap<String, String>>,
    #[schemars(description = "Execution timeout in milliseconds")]
    pub timeout_ms: Option<u64>,
    #[schemars(description = "Maximum bytes returned for each stdout and stderr stream")]
    pub max_output_bytes: Option<usize>,
}
