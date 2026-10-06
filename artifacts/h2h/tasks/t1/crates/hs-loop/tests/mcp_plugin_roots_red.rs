//! RED: hs-plugin-mcpcall's PLUGIN mode - the path the kernel uses for
//! every MCP tool call - passed model args straight to the MCP server
//! with NO `allowed_roots` pre-check; the CLI's --path-args gate existed
//! only on the interactive path (found in the 2026-09-09 deep pass).
//! Plugin mode now enforces `allowed_roots` on path-like args BEFORE the
//! server is spawned (deny by default); the server-side contract stays
//! as defense in depth, not the only line.

use std::io::{BufRead, BufReader, Write as _};
use std::process::{Command, Stdio};

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

/// One plugin-mode tool.call over the JSON-lines wire protocol.
fn plugin_call(cfg: &std::path::Path, args_json: &str) -> serde_json::Value {
    let mut child = Command::new(BIN)
        .args([
            "--plugin",
            "--config",
            cfg.to_str().unwrap(),
            "--server",
            "fixture",
            "--tool",
            "echo",
            "--name",
            "mcp.fixture.echo",
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let line = format!("{{\"id\":1,\"method\":\"tool.call\",\"params\":{{\"args\":{args_json}}}}}\n");
    child
        .stdin
        .as_mut()
        .unwrap()
        .write_all(line.as_bytes())
        .unwrap();
    drop(child.stdin.take()); // EOF: one call, then exit
    let out = child.stdout.take().unwrap();
    let line = BufReader::new(out)
        .lines()
        .next()
        .expect("one response line")
        .unwrap();
    let _ = child.wait();
    serde_json::from_str(&line).unwrap()
}

/// P1 (RED): an absolute path outside `allowed_roots` is refused BEFORE
/// the server sees it - pre-fix the server was reached and echoed.
#[test]
fn p1_absolute_path_outside_roots_refused() {
    let cfg = config();
    let resp = plugin_call(cfg.path(), "{\"text\":\"/etc/passwd\"}");
    let err = resp["error"].as_str().unwrap_or("");
    assert!(
        err.contains("allowed_roots") && err.contains("/etc/passwd"),
        "plugin mode must refuse pre-spawn: {resp}"
    );
}

/// P2: an in-root path still reaches the server.
#[test]
fn p2_in_root_path_passes() {
    let cfg = config();
    let resp = plugin_call(cfg.path(), "{\"text\":\"/ws/data.txt\"}");
    let s = serde_json::to_string(&resp).unwrap();
    assert!(s.contains("/ws/data.txt"), "echo payload: {s}");
}

/// P3: non-path strings are unaffected (regression pin).
#[test]
fn p3_plain_string_passes() {
    let cfg = config();
    let resp = plugin_call(cfg.path(), "{\"text\":\"hello-mcp\"}");
    let s = serde_json::to_string(&resp).unwrap();
    assert!(s.contains("hello-mcp"), "echo payload: {s}");
}

/// P4: URLs contain slashes but are not filesystem paths - unaffected.
#[test]
fn p4_url_string_passes() {
    let cfg = config();
    let resp = plugin_call(cfg.path(), "{\"text\":\"https://example.com/a/b\"}");
    let s = serde_json::to_string(&resp).unwrap();
    assert!(s.contains("https://example.com/a/b"), "echo payload: {s}");
}

/// P5 (RED): traversal-style relative paths are refused pre-spawn.
#[test]
fn p5_traversal_refused() {
    let cfg = config();
    let resp = plugin_call(cfg.path(), "{\"text\":\"../escape\"}");
    let err = resp["error"].as_str().unwrap_or("");
    assert!(err.contains("allowed_roots"), "traversal refused: {resp}");
}

/// P6 (RED): path-like values ANYWHERE in the arg tree are refused
/// pre-spawn (an attacker nests where a flat scan would miss).
#[test]
fn p6_nested_path_refused() {
    let cfg = config();
    let resp = plugin_call(cfg.path(), "{\"text\":\"hello\",\"extra\":[\"/etc/shadow\"]}");
    let err = resp["error"].as_str().unwrap_or("");
    assert!(
        err.contains("allowed_roots") && err.contains("/etc/shadow"),
        "nested path refused: {resp}"
    );
}
