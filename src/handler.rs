use rmcp::{ServerHandler, tool_handler};

use crate::workspace::FileSystemServer;

fn all_tools() -> rmcp::handler::server::router::tool::ToolRouter<FileSystemServer> {
    let mut router = FileSystemServer::workspace_router();
    router.merge(FileSystemServer::filesystem_router());
    router.merge(FileSystemServer::exec_router());
    router
}

#[tool_handler(router = all_tools())]
impl ServerHandler for FileSystemServer {}
