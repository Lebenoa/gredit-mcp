use std::{fs, io, path::Path};

use rmcp::{ErrorData as McpError, handler::server::wrapper::Parameters, tool, tool_router};

use crate::{
    shared::{
        DEFAULT_MAX_READ_BYTES, DEFAULT_MAX_RESULTS, MAX_GREP_SCAN_BYTES, MAX_GREP_SCAN_FILES,
        MAX_READ_BYTES, MAX_RESULTS, io_tool_error, result_limit, tool_error,
    },
    types::{EditRequest, GrepRequest, ListRequest, ReadRequest, WriteRequest},
    workspace::{FileSystemServer, display_relative},
};

#[tool_router(router = filesystem_router, vis = "pub(crate)")]
impl FileSystemServer {
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

    #[tool(description = "List workspace files and directories")]
    pub fn list(&self, Parameters(request): Parameters<ListRequest>) -> Result<String, McpError> {
        let path = self
            .resolve_directory(request.path.as_deref().unwrap_or("."))
            .map_err(tool_error)?;
        let max_entries = result_limit(
            request.max_entries,
            "max_entries",
            DEFAULT_MAX_RESULTS,
            MAX_RESULTS,
        )
        .map_err(tool_error)?;
        let root = self.root();
        let mut entries = Vec::new();
        collect_entries(
            &root,
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
        let max_results = result_limit(
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
        let root = self.root();
        let mut matches = Vec::new();
        let mut scanned_files = 0usize;
        let mut scanned_bytes = 0u64;
        search_path(
            &root,
            &path,
            suffix,
            &regex,
            max_results,
            &mut matches,
            &mut scanned_files,
            &mut scanned_bytes,
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

#[allow(clippy::too_many_arguments)] // recursive helper threading scan state
fn search_path(
    root: &Path,
    path: &Path,
    suffix: Option<&str>,
    regex: &regex::Regex,
    max_results: usize,
    matches: &mut Vec<String>,
    scanned_files: &mut usize,
    scanned_bytes: &mut u64,
) -> io::Result<()> {
    if matches.len() >= max_results {
        return Ok(());
    }
    let metadata = fs::symlink_metadata(path)?;
    // Never follow symlinks, neither as directories nor as files.
    if metadata.file_type().is_symlink() {
        return Ok(());
    }
    if metadata.is_dir() {
        let mut children = fs::read_dir(path)?.collect::<Result<Vec<_>, _>>()?;
        children.sort_unstable_by_key(|entry| entry.file_name());
        for entry in children {
            search_path(
                root,
                &entry.path(),
                suffix,
                regex,
                max_results,
                matches,
                scanned_files,
                scanned_bytes,
            )?;
            if matches.len() >= max_results || *scanned_files >= MAX_GREP_SCAN_FILES {
                break;
            }
        }
        return Ok(());
    }
    if !metadata.is_file() || suffix.is_some_and(|suffix| !path.to_string_lossy().ends_with(suffix))
    {
        return Ok(());
    }
    if *scanned_files >= MAX_GREP_SCAN_FILES {
        return Ok(());
    }
    *scanned_files += 1;
    let size = metadata.len();
    if *scanned_bytes + size > MAX_GREP_SCAN_BYTES {
        return Ok(());
    }
    *scanned_bytes += size;
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
