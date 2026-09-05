//! Client-side MCP bridge, pure parts (docs/mcp-adapter-gate.md): config
//! schema, tool namespacing, allowed-roots path enforcement. The async
//! server lifecycle (rmcp client, spawn/kill per mission, discovery ->
//! kernel registry) lands in hs-plugin-mcpbridge; the kernel stays
//! untouched and every call routes through kernel.call_tool so ToolCall
//! audit events hold automatically.

use std::path::{Component, Path};

#[derive(Clone, Debug, serde::Deserialize)]
pub struct McpServerConfig {
    pub name: String,
    pub command: Vec<String>,
    #[serde(default)]
    pub allowed_roots: Vec<String>,
}

#[derive(serde::Deserialize)]
struct McpFile {
    #[serde(default)]
    mcp_servers: Vec<McpServerConfig>,
}

pub fn load_mcp_servers(path: &Path) -> Result<Vec<McpServerConfig>, String> {
    let text = std::fs::read_to_string(path)
        .map_err(|e| format!("mcp servers config {}: {e}", path.display()))?;
    let f: McpFile =
        toml::from_str(&text).map_err(|e| format!("mcp servers config {}: {e}", path.display()))?;
    Ok(f.mcp_servers)
}

/// Every MCP tool reaches the model namespaced by server:
/// mcp.<server>.<tool>. No bare names - a server cannot shadow a builtin.
pub fn namespaced_tool(server: &str, tool: &str) -> String {
    format!("mcp.{server}.{tool}")
}

/// Normalize without touching the fs (the path may not exist yet): resolve
/// `.`/`..` lexically, then require membership under an allowed root.
/// Deny by default: empty allowed_roots rejects everything.
pub fn check_path_allowed(cfg: &McpServerConfig, path: &str) -> Result<(), String> {
    let mut norm: Vec<std::ffi::OsString> = Vec::new();
    for c in Path::new(path).components() {
        match c {
            Component::CurDir => {}
            Component::ParentDir => {
                norm.pop();
            }
            Component::RootDir => norm.clear(),
            other => norm.push(other.as_os_str().to_os_string()),
        }
    }
    let mut normalized = String::from("/");
    normalized.push_str(
        &norm
            .iter()
            .map(|s| s.to_string_lossy().into_owned())
            .collect::<Vec<_>>()
            .join("/"),
    );
    for root in &cfg.allowed_roots {
        let root = root.trim_end_matches('/');
        if normalized == root || normalized.starts_with(&format!("{root}/")) {
            return Ok(());
        }
    }
    Err(format!(
        "mcp.{}: path {path} outside allowed_roots {:?} (deny by default)",
        cfg.name, cfg.allowed_roots
    ))
}

/// Resolve a plugin binary as the canonicalized sibling of the running
/// executable. B4 lesson: a bridge silently picked up from another tree
/// runs stale code, so a missing sibling is a LOUD error - never a
/// fallback to PATH or a neighboring checkout.
pub fn resolve_plugin_bin(exe: &Path, name: &str) -> Result<std::path::PathBuf, String> {
    let exe = exe
        .canonicalize()
        .map_err(|e| format!("resolve {name}: exe {}: {e}", exe.display()))?;
    let dir = exe
        .parent()
        .ok_or_else(|| format!("resolve {name}: exe {} has no parent dir", exe.display()))?;
    let cand = dir.join(name);
    if cand.exists() {
        Ok(cand)
    } else {
        Err(format!(
            "mcp bridge binary {name} not found next to {} (looked at {}) - build it in the same tree; no fallback to PATH or other trees",
            exe.display(),
            cand.display()
        ))
    }
}
