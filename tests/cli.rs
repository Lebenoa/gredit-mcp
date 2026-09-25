use std::process::Command;

/// Transport flags are accepted; unrecognized flags are rejected up front.
#[test]
fn cli_rejects_unknown_flags() {
    let workspace = std::env::temp_dir();
    let output = Command::new(env!("CARGO_BIN_EXE_gredit-mcp"))
        .args(["--bogus"])
        .arg(&workspace)
        .output()
        .expect("run binary");
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("unknown flag"), "stderr: {stderr}");
}

/// Without transport flags the server must still start on stdio; a missing
/// workspace error is fine, but an unknown flag must never be accepted.
#[test]
fn cli_accepts_remote_exec_flag() {
    let output = Command::new(env!("CARGO_BIN_EXE_gredit-mcp"))
        .args(["--http", "--allow-remote-exec", "--addr", "not-an-address"])
        .output()
        .expect("run binary");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(!stderr.contains("unknown flag"), "stderr: {stderr}");
    assert!(
        stderr.contains("invalid socket address"),
        "stderr: {stderr}"
    );
}

/// Without transport flags the server must still accept the workspace positional.
#[test]
fn cli_accepts_workspace_positional() {
    let output = Command::new(env!("CARGO_BIN_EXE_gredit-mcp"))
        .arg(env!("CARGO_MANIFEST_DIR"))
        .output()
        .expect("run binary");
    // stdio mode blocks waiting for JSON-RPC on stdin; the process is killed by
    // the harness once stdin closes, so any status is acceptable — the point is
    // it must not fail with a flag-parse error.
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(!stderr.contains("unknown flag"), "stderr: {stderr}");
}
