//! RED contract tests: hs-plugin-mcpcall - the per-call MCP bridge.
//! Spawns the named server (child-process transport), handshakes via rmcp,
//! lists or calls one tool, kills the child. Audit stays automatic because
//! the kernel invokes this bin per tool call (`ToolCall` events). Fixture:
//! in-tree hs-mcp-fixture server with one tool, echo.

use std::process::Command;

const BIN: &str = env!("CARGO_BIN_EXE_hs-plugin-mcpcall");
const FIXTURE: &str = env!("CARGO_BIN_EXE_hs-mcp-fixture");

fn config() -> tempfile::NamedTempFile {
    let f = tempfile::NamedTempFile::new().unwrap();
    std::fs::write(
        f.path(),
        format!(
            "[[mcp_servers]]\nname = \"fixture\"\ncommand = [\"{FIXTURE}\"]\nallowed_roots = [\"/ws\"]\n"
        ),
    )
    .unwrap();
    f
}

#[test]
fn list_discovers_namespaced_tools() {
    let cfg = config();
    let out = Command::new(BIN)
        .args([
            "--config",
            cfg.path().to_str().unwrap(),
            "--server",
            "fixture",
            "--list",
        ])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let tools: Vec<String> = serde_json::from_slice(&out.stdout).unwrap();
    assert!(
        tools.contains(&"mcp.fixture.echo".to_string()),
        "got {tools:?}"
    );
}

#[test]
fn call_round_trips_through_real_mcp_protocol() {
    let cfg = config();
    let out = Command::new(BIN)
        .args([
            "--config",
            cfg.path().to_str().unwrap(),
            "--server",
            "fixture",
            "--call",
            "echo",
            "--args",
            "{\"text\":\"hello-mcp\"}",
        ])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    let s = serde_json::to_string(&v).unwrap();
    assert!(s.contains("hello-mcp"), "echo payload: {s}");
}

#[test]
fn unknown_server_and_unknown_tool_error_cleanly() {
    let cfg = config();
    let out = Command::new(BIN)
        .args([
            "--config",
            cfg.path().to_str().unwrap(),
            "--server",
            "nope",
            "--list",
        ])
        .output()
        .unwrap();
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("nope"));
    let out = Command::new(BIN)
        .args([
            "--config",
            cfg.path().to_str().unwrap(),
            "--server",
            "fixture",
            "--call",
            "nonexistent",
            "--args",
            "{}",
        ])
        .output()
        .unwrap();
    assert!(!out.status.success(), "unknown tool must fail");
}

#[test]
fn path_args_outside_allowed_roots_are_rejected_before_spawning() {
    let cfg = config();
    let out = Command::new(BIN)
        .args([
            "--config",
            cfg.path().to_str().unwrap(),
            "--server",
            "fixture",
            "--call",
            "echo",
            "--args",
            "{\"text\":\"/etc/passwd\"}",
            "--path-args",
            "text",
        ])
        .output()
        .unwrap();
    assert!(!out.status.success(), "path outside roots must fail");
    assert!(String::from_utf8_lossy(&out.stderr).contains("/etc/passwd"));
}
