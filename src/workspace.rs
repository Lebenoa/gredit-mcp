use std::{
    fs, io,
    path::{Path, PathBuf},
    sync::RwLock,
};

use rmcp::{
    RoleServer, handler::server::wrapper::Parameters, service::RequestContext, tool, tool_router,
};

use crate::{
    shared::{tool_error, validate_relative_path},
    types::{SetWorkspaceRequest, WorkspaceApproval},
};

#[derive(Debug)]
pub struct FileSystemServer {
    pub(crate) root: RwLock<PathBuf>,
    pub(crate) allow_exec: bool,
}

impl FileSystemServer {
    /// Create workspace-only server. `exec` remains disabled.
    pub fn new(root: impl AsRef<Path>) -> io::Result<Self> {
        Self::from_root(root, false)
    }

    /// Create server for trusted stdio use where Nushell execution is explicitly allowed.
    pub fn new_with_exec(root: impl AsRef<Path>) -> io::Result<Self> {
        Self::from_root(root, true)
    }

    fn from_root(root: impl AsRef<Path>, allow_exec: bool) -> io::Result<Self> {
        let root = canonical_directory(root)?;
        Ok(Self {
            root: RwLock::new(root),
            allow_exec,
        })
    }

    pub fn root(&self) -> PathBuf {
        self.root
            .read()
            .expect("workspace root lock poisoned")
            .clone()
    }

    pub(crate) fn set_root(&self, root: impl AsRef<Path>) -> io::Result<()> {
        let root = canonical_directory(root)?;
        *self.root.write().expect("workspace root lock poisoned") = root;
        Ok(())
    }

    pub(crate) fn resolve_existing(&self, input: &str) -> Result<PathBuf, String> {
        let relative = validate_relative_path(input)?;
        let root = self.root();
        let path = root.join(relative);
        let resolved = fs::canonicalize(&path)
            .map_err(|error| format!("cannot resolve '{}': {error}", input))?;
        ensure_inside_root(&root, &resolved)?;
        Ok(resolved)
    }

    pub(crate) fn resolve_for_create(&self, input: &str) -> Result<PathBuf, String> {
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
        ensure_inside_root(&root, &resolved)?;
        for component in missing.iter().rev() {
            resolved.push(component);
        }
        Ok(resolved)
    }

    pub(crate) fn resolve_directory(&self, input: &str) -> Result<PathBuf, String> {
        let path = self.resolve_existing(input)?;
        if path.is_dir() {
            Ok(path)
        } else {
            Err(format!("'{}' is not a directory", input))
        }
    }

    pub(crate) fn display_path(&self, path: &Path) -> String {
        display_relative(&self.root(), path)
    }
}

#[tool_router(router = workspace_router, vis = "pub(crate)")]
impl FileSystemServer {
    #[tool(
        description = "Request user approval and switch the active workspace root to an absolute directory"
    )]
    pub async fn set_workspace(
        &self,
        context: RequestContext<RoleServer>,
        Parameters(request): Parameters<SetWorkspaceRequest>,
    ) -> Result<String, rmcp::ErrorData> {
        let requested_input = Path::new(&request.path);
        if !requested_input.is_absolute() {
            return Err(tool_error("workspace path must be absolute"));
        }
        let requested = canonical_directory(requested_input)
            .map_err(|error| tool_error(format!("cannot resolve requested workspace: {error}")))?;
        let requested = requested.to_string_lossy().into_owned();
        let Some(approval) = context
            .peer
            .elicit::<WorkspaceApproval>(format!(
                "Allow this server to switch its workspace to '{}'? This changes the directory used by read, write, edit, list, grep, and trusted exec when exec is enabled.",
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
}

fn canonical_directory(path: impl AsRef<Path>) -> io::Result<PathBuf> {
    let path = fs::canonicalize(path)?;
    if path.is_dir() {
        Ok(path)
    } else {
        Err(io::Error::new(
            io::ErrorKind::NotADirectory,
            format!("workspace root is not a directory: {}", path.display()),
        ))
    }
}

fn ensure_inside_root(root: &Path, path: &Path) -> Result<(), String> {
    if path == root || path.starts_with(root) {
        Ok(())
    } else {
        Err(format!(
            "path '{}' is outside the workspace root",
            path.display()
        ))
    }
}

pub(crate) fn display_relative(root: &Path, path: &Path) -> String {
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

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

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
}
