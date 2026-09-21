//! Structured tool output types.
//!
//! Each tool returns `Json<T>`; `T` is serialized into the tool result's
//! `structuredContent` and its `JsonSchema` implementation is advertised as
//! the tool's `outputSchema` in `tools/list`.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ReadOutput {
    #[schemars(description = "Workspace-relative file path")]
    pub path: String,
    #[schemars(description = "Total physical lines in the file")]
    pub total_lines: usize,
    #[schemars(description = "Requested 1-based line range")]
    pub lines: Vec<LineOutput>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct LineOutput {
    #[schemars(description = "1-based line number")]
    pub number: usize,
    #[schemars(description = "Line text without the trailing newline")]
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct WriteOutput {
    #[schemars(description = "Workspace-relative file path")]
    pub path: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct EditOutput {
    #[schemars(description = "Workspace-relative file path")]
    pub path: String,
    #[schemars(description = "Number of replacements applied")]
    pub replacements: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ListOutput {
    #[schemars(description = "Workspace-relative directory path")]
    pub path: String,
    #[schemars(description = "Entries sorted by name, directories before files")]
    pub entries: Vec<ListEntryOutput>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ListEntryOutput {
    #[schemars(description = "'dir' or 'file'")]
    pub kind: String,
    #[schemars(description = "Workspace-relative entry path")]
    pub path: String,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, JsonSchema)]
pub struct GrepOutput {
    #[schemars(description = "Matching lines sorted by path then line number")]
    pub matches: Vec<GrepMatchOutput>,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, JsonSchema)]
pub struct GrepMatchOutput {
    #[schemars(description = "Workspace-relative file path")]
    pub path: String,
    #[schemars(description = "1-based matching line number")]
    pub line: usize,
    #[schemars(description = "Matching line text")]
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ExecOutput {
    #[schemars(description = "Always 'embedded-nushell'")]
    pub engine: String,
    #[schemars(description = "Workspace-relative working directory used for evaluation")]
    pub working_dir: String,
    #[schemars(description = "Exit status of the evaluation (0 for embedded evaluation)")]
    pub exit_code: i32,
    #[schemars(description = "Captured stdout and stderr text")]
    pub stdout: String,
    #[schemars(description = "True when stdout was truncated to max_output_bytes")]
    pub output_truncated: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct SetWorkspaceOutput {
    #[schemars(description = "Absolute workspace root path after the change")]
    pub path: String,
}
