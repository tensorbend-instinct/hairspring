//! RED contract tests: client-side MCP bridge (the MCP adapter gate design).
//! Kernel spawns the adapter; adapter spawns one MCP server per mission;
//! tools appear namespaced mcp.<server>.<tool>; every call routes through
//! kernel.call_tool so ToolCall audit events hold automatically. Pure-part
//! contracts here: config schema, namespacing, allowed-roots enforcement.

use hs_loop::mcpbridge::*;

const CFG: &str = r#"
[[mcp_servers]]
name = "fs"
command = ["npx", "-y", "@modelcontextprotocol/server-filesystem", "/ws"]
allowed_roots = ["/ws"]

[[mcp_servers]]
name = "git"
command = ["uvx", "mcp-server-git", "--repository", "/ws"]
allowed_roots = ["/ws", "/tmp/scratch"]
"#;

#[test]
fn parses_mcp_servers_config() {
    let f = tempfile::NamedTempFile::new().unwrap();
    std::fs::write(f.path(), CFG).unwrap();
    let servers = load_mcp_servers(f.path()).unwrap();
    assert_eq!(servers.len(), 2);
    assert_eq!(servers[0].name, "fs");
    assert_eq!(servers[0].command[0], "npx");
    assert_eq!(servers[0].allowed_roots, vec!["/ws".to_string()]);
    assert_eq!(servers[1].allowed_roots.len(), 2);
}

#[test]
fn malformed_mcp_config_errors() {
    let f = tempfile::NamedTempFile::new().unwrap();
    std::fs::write(f.path(), "[[mcp_servers]]\nname = 7\n").unwrap();
    assert!(load_mcp_servers(f.path()).is_err());
}

#[test]
fn tool_names_are_namespaced_per_server() {
    assert_eq!(namespaced_tool("fs", "read_file"), "mcp.fs.read_file");
    assert_eq!(namespaced_tool("git", "log"), "mcp.git.log");
}

#[test]
fn allowed_roots_enforced_on_path_args() {
    let f = tempfile::NamedTempFile::new().unwrap();
    std::fs::write(f.path(), CFG).unwrap();
    let servers = load_mcp_servers(f.path()).unwrap();
    let fs = &servers[0];
    assert!(check_path_allowed(fs, "/ws/src/main.rs").is_ok());
    assert!(check_path_allowed(fs, "/ws").is_ok());
    let e = check_path_allowed(fs, "/etc/passwd").unwrap_err();
    assert!(e.contains("/etc/passwd"), "names the rejected path: {e}");
    // prefix-trick rejection: /ws2 is NOT under /ws
    assert!(check_path_allowed(fs, "/ws2/evil").is_err(), "prefix trick");
    // git server allows /tmp/scratch too
    assert!(check_path_allowed(&servers[1], "/tmp/scratch/x").is_ok());
}

#[test]
fn empty_allowed_roots_means_no_paths() {
    // A server with no allowed roots must not receive any path - the
    // sandbox default is deny, matching the isolation-only posture.
    let f = tempfile::NamedTempFile::new().unwrap();
    std::fs::write(
        f.path(),
        "[[mcp_servers]]\nname = \"noop\"\ncommand = [\"true\"]\nallowed_roots = []\n",
    )
    .unwrap();
    let servers = load_mcp_servers(f.path()).unwrap();
    assert!(check_path_allowed(&servers[0], "/ws/x").is_err());
}
