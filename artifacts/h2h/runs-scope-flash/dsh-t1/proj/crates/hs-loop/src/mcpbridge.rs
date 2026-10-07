//! Client-side MCP bridge, pure parts (the MCP adapter gate design): config
//! schema, tool namespacing, allowed-roots path enforcement. The async
//! server lifecycle (rmcp client, spawn/kill per mission, discovery ->
//! kernel registry) lands in hs-plugin-mcpbridge; the kernel stays
//! untouched and every call routes through `kernel.call_tool` so `ToolCall`
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
#[must_use]
pub fn namespaced_tool(server: &str, tool: &str) -> String {
    format!("mcp.{server}.{tool}")
}

/// Normalize without touching the fs (the path may not exist yet): resolve
/// `.`/`..` lexically, then require membership under an allowed root.
/// Deny by default: empty `allowed_roots` rejects everything.
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

/// Plugin-mode equivalent of the CLI's `--path-args` gate: with no
/// operator naming the path arguments per call, every string ANYWHERE in
/// the arg tree that looks like a path must pass the same `allowed_roots`
/// check BEFORE the server is spawned (deny by default). The server-side
/// contract stays as defense in depth, not the only line (deep pass
/// 2026-09-09: plugin mode had no client-side gate at all).
pub fn check_args_paths(cfg: &McpServerConfig, args: &serde_json::Value) -> Result<(), String> {
    fn walk(cfg: &McpServerConfig, v: &serde_json::Value, depth: u32) -> Result<(), String> {
        if depth > 32 {
            return Ok(());
        }
        match v {
            serde_json::Value::String(s) => {
                if is_path_like(s) {
                    check_path_allowed(cfg, s)?;
                }
                Ok(())
            }
            serde_json::Value::Array(a) => {
                for x in a {
                    walk(cfg, x, depth + 1)?;
                }
                Ok(())
            }
            serde_json::Value::Object(o) => {
                for x in o.values() {
                    walk(cfg, x, depth + 1)?;
                }
                Ok(())
            }
            _ => Ok(()),
        }
    }
    walk(cfg, args, 0)
}

/// A string is path-like when it is absolute or climbs with `..`. URLs
/// (`scheme://`) are not filesystem paths, and bare names the server
/// resolves against its own cwd carry no client-verifiable root.
fn is_path_like(s: &str) -> bool {
    if s.contains("://") {
        return false;
    }
    s.starts_with('/') || s == ".." || s.starts_with("../") || s.ends_with("/..") || s.contains("/../")
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

/// Discovery shared by every runner seam (hs-swe-run's block generalized for
/// hs-tb-run and the REPL, Eric 2026-09-07 web-tooling order): for each
/// server in the TOML, list its tools through hs-plugin-mcpcall and return
/// (kernel config fragment, native tool schemas with the server-provided
/// `input_schema` verbatim). Discovery failure is a hard error - a
/// half-registered surface is worse than none.
///
/// Binary resolution: `HS_MCP_BRIDGE_BIN` wins when set (test seam - the
/// REPL runs discovery in-process, where `current_exe` is the test binary,
/// not a sibling of the plugins); otherwise `resolve_plugin_bin` next to the
/// current exe (production: every runner ships its plugins beside it).
pub fn discover_mcp_tools(
    servers_toml: &Path,
) -> Result<(String, Vec<serde_json::Value>), String> {
    let servers = load_mcp_servers(servers_toml)?;
    let mcpcall = match std::env::var("HS_MCP_BRIDGE_BIN") {
        Ok(p) => std::path::PathBuf::from(p),
        Err(_) => resolve_plugin_bin(
            &std::env::current_exe().map_err(|e| format!("current exe: {e}"))?,
            "hs-plugin-mcpcall",
        )?,
    };
    let mut fragment = String::new();
    let mut native = Vec::new();
    for s in &servers {
        let out = std::process::Command::new(&mcpcall)
            .args([
                "--config",
                &servers_toml.display().to_string(),
                "--server",
                &s.name,
                "--list",
                "--list-verbose",
            ])
            .output()
            .map_err(|e| format!("mcp discovery spawn: {e}"))?;
        if !out.status.success() {
            return Err(format!(
                "mcp discovery: {}",
                String::from_utf8_lossy(&out.stderr)
            ));
        }
        let discovered: Vec<serde_json::Value> = serde_json::from_slice(&out.stdout)
            .map_err(|e| format!("mcp discovery parse: {e}"))?;
        for d in &discovered {
            let full = d["name"].as_str().unwrap_or("").to_string();
            let desc = d["description"].as_str().unwrap_or("").to_string();
            let tool = full
                .strip_prefix(&format!("mcp.{}.", s.name))
                .ok_or_else(|| format!("unexpected tool name {full}"))?
                .to_string();
            fragment.push_str(&format!(
                "\n[[tools]]\nname = \"{full}\"\ncommand = [\"{}\", \"--plugin\", \"--config\", \"{}\", \"--server\", \"{}\", \"--tool\", \"{tool}\", \"--name\", \"{full}\"]\nsubjects = [\"*\"]\n",
                mcpcall.display(),
                servers_toml.display(),
                s.name,
            ));
            native.push(crate::toolschema::mcp_tool(
                &full,
                &desc,
                d.get("input_schema").cloned(),
            ));
        }
    }
    Ok((fragment, native))
}
