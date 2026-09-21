use std::fs;

use rmcp::handler::server::wrapper::Parameters;
use tempfile::tempdir;

use crate::{
    EditRequest, ExecRequest, FileSystemServer, GrepRequest, ListRequest, ReadRequest, WriteRequest,
};

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

#[tokio::test]
async fn exec_runs_in_workspace_with_embedded_nushell() {
    let (_directory, server) = server();
    let result = server
        .exec(Parameters(ExecRequest {
            command: "echo hello".to_owned(),
            working_dir: None,
            env: None,
            timeout_ms: Some(5_000),
            max_output_bytes: Some(1_024),
        }))
        .await
        .expect("exec");
    assert!(result.contains("engine: embedded-nushell"));
    assert!(result.contains("exit_code: 0"));
    assert!(result.contains("hello"));
}

#[tokio::test]
async fn exec_evaluates_nushell_pipeline() {
    let (_directory, server) = server();
    let result = server
        .exec(Parameters(ExecRequest {
            command: "[3 1 2] | sort | str join ','".to_owned(),
            working_dir: None,
            env: None,
            timeout_ms: Some(5_000),
            max_output_bytes: Some(1_024),
        }))
        .await
        .expect("pipeline exec");
    assert!(result.contains("1,2,3"));
}

#[tokio::test]
async fn exec_rejects_outside_working_directory() {
    let (_directory, server) = server();
    let error = server
        .exec(Parameters(ExecRequest {
            command: "echo hello".to_owned(),
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
