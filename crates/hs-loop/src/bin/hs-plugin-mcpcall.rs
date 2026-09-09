//! Per-call MCP bridge (client side, the MCP adapter gate design): spawns the
//! named MCP server as a child process, handshakes via rmcp, lists or calls
//! one tool, exits (child dies with us). The kernel invokes this bin per
//! tool call, so ToolCall audit events hold automatically.
//!
//! CLI:
//!   hs-plugin-mcpcall --config <toml> --server <name> --list
//!   hs-plugin-mcpcall --config <toml> --server <name> --call <tool> \
//!       --args '<json>' [--path-args a,b]
//! --path-args: argument names whose string values must pass the server's
//! allowed_roots check before the server is even spawned (deny by default).

use hs_loop::mcpbridge::*;
use rmcp::{model::CallToolRequestParam, transport::TokioChildProcess, ServiceExt};

fn has_flag(args: &[String], flag: &str) -> bool {
    args.iter().any(|a| a == flag)
}

fn arg<'a>(args: &'a [String], flag: &str) -> Option<&'a str> {
    args.iter()
        .position(|a| a == flag)
        .and_then(|i| args.get(i + 1))
        .map(String::as_str)
}

fn fail(msg: String) -> ! {
    eprintln!("hs-plugin-mcpcall: {msg}");
    std::process::exit(2)
}

include!("shared/sdk.rs");

/// Plugin mode: the kernel spawns one process per discovered tool
/// (--plugin --name mcp.<server>.<tool> --server <s> --tool <t> --config
/// <toml>); each tool.call spawns the MCP server, calls, returns, exits.
fn plugin_main(cfg_path: &str, server_name: &str, tool: &str, full_name: &str) {
    let leaked: &'static str = Box::leak(full_name.to_string().into_boxed_str());
    let cfg_path = cfg_path.to_string();
    let server_name = server_name.to_string();
    let tool = tool.to_string();
    // One runtime for the whole plugin lifetime, built HERE (plugin_main is
    // called before any runtime context exists). Building a fresh runtime
    // per call inside #[tokio::main] panicked with "cannot start a runtime
    // from within a runtime" - the MCP tool surface was silently dead until
    // the supervisor surfaced the EOF (found 2026-09-05, phase 1).
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    serve(leaked, "tool", &mut move |method, params| {
        if method != "tool.call" {
            return serde_json::json!({"$error": "unknown method"});
        }
        let args = params["args"].clone();
        match rt.block_on(mcp_call(&cfg_path, &server_name, &tool, args)) {
            Ok(v) => v,
            Err(e) => serde_json::json!({"$error": e}),
        }
    });
}

async fn mcp_call(
    cfg_path: &str,
    server_name: &str,
    tool: &str,
    args: serde_json::Value,
) -> Result<serde_json::Value, String> {
    let servers = load_mcp_servers(std::path::Path::new(cfg_path))?;
    let cfg = servers
        .iter()
        .find(|s| s.name == server_name)
        .ok_or_else(|| format!("unknown server '{server_name}'"))?
        .clone();
    let mut cmd = tokio::process::Command::new(&cfg.command[0]);
    cmd.args(&cfg.command[1..]);
    let service =
        ().serve(TokioChildProcess::new(cmd).map_err(|e| format!("spawn: {e}"))?)
            .await
            .map_err(|e| format!("mcp handshake: {e}"))?;
    let obj = args.as_object().cloned().unwrap_or_default();
    let result = service
        .call_tool(CallToolRequestParam {
            name: tool.to_string().into(),
            arguments: Some(obj),
        })
        .await
        .map_err(|e| format!("call_tool {tool}: {e}"))?;
    Ok(serde_json::json!({
        "content": result.content,
        "is_error": result.is_error.unwrap_or(false),
    }))
}

fn main() {
    // Sync main: the CLI paths build their own runtime below, and
    // plugin_main builds its own - no shared runtime context, no nesting.
    real_main();
}

fn real_main() {
    let argv: Vec<String> = std::env::args().collect();
    let cfg_path = arg(&argv, "--config").unwrap_or_else(|| fail("--config required".into()));
    let server_name = arg(&argv, "--server").unwrap_or_else(|| fail("--server required".into()));
    if has_flag(&argv, "--plugin") {
        let full = arg(&argv, "--name").unwrap_or_else(|| fail("--name required".into()));
        let tool = arg(&argv, "--tool").unwrap_or_else(|| fail("--tool required".into()));
        plugin_main(cfg_path, server_name, tool, full);
        return;
    }
    let servers = load_mcp_servers(std::path::Path::new(cfg_path)).unwrap_or_else(|e| fail(e));
    let cfg = servers
        .iter()
        .find(|s| s.name == server_name)
        .unwrap_or_else(|| {
            fail(format!(
                "unknown server '{server_name}' (not in {cfg_path})"
            ))
        })
        .clone();

    // path-arg enforcement BEFORE spawning the server (defense in depth)
    if let Some(call_tool) = arg(&argv, "--call") {
        let _ = call_tool;
        let raw = arg(&argv, "--args").unwrap_or("{}");
        let parsed: serde_json::Value =
            serde_json::from_str(raw).unwrap_or_else(|e| fail(format!("bad --args json: {e}")));
        if let Some(list) = arg(&argv, "--path-args") {
            for key in list.split(',').map(|k| k.trim()).filter(|k| !k.is_empty()) {
                if let Some(v) = parsed.get(key).and_then(|v| v.as_str()) {
                    check_path_allowed(&cfg, v).unwrap_or_else(|e| fail(e));
                }
            }
        }
    }

    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap_or_else(|e| fail(format!("runtime: {e}")));
    rt.block_on(cli_main(argv, cfg));
}

async fn cli_main(argv: Vec<String>, cfg: McpServerConfig) {
    let mut cmd = tokio::process::Command::new(&cfg.command[0]);
    cmd.args(&cfg.command[1..]);
    let service =
        ().serve(TokioChildProcess::new(cmd).unwrap_or_else(|e| fail(format!("spawn: {e}"))))
            .await
            .unwrap_or_else(|e| fail(format!("mcp handshake: {e}")));

    if has_flag(&argv, "--list") {
        let tools = service
            .list_all_tools()
            .await
            .unwrap_or_else(|e| fail(format!("list_tools: {e}")));
        let verbose = has_flag(&argv, "--list-verbose");
        if verbose {
            let full: Vec<serde_json::Value> = tools
                .iter()
                .map(|t| {
                    serde_json::json!({
                        "name": namespaced_tool(&cfg.name, &t.name),
                        "description": t.description.as_deref().unwrap_or(""),
                        // native tool delivery: the server's own input schema,
                        // verbatim from tools/list
                        "input_schema": serde_json::to_value(&t.input_schema)
                            .unwrap_or(serde_json::json!({"type":"object","properties":{}})),
                    })
                })
                .collect();
            println!("{}", serde_json::to_string(&full).unwrap());
        } else {
            let names: Vec<String> = tools
                .iter()
                .map(|t| namespaced_tool(&cfg.name, &t.name))
                .collect();
            println!("{}", serde_json::to_string(&names).unwrap());
        }
        return;
    }
    let tool = arg(&argv, "--call").unwrap_or_else(|| fail("--list or --call required".into()));
    let raw = arg(&argv, "--args").unwrap_or("{}");
    let parsed: serde_json::Value =
        serde_json::from_str(raw).unwrap_or_else(|e| fail(format!("bad --args json: {e}")));
    let obj = parsed.as_object().cloned().unwrap_or_default();
    let result = service
        .call_tool(CallToolRequestParam {
            name: tool.to_string().into(),
            arguments: Some(obj),
        })
        .await
        .unwrap_or_else(|e| fail(format!("call_tool {tool}: {e}")));
    println!(
        "{}",
        serde_json::to_string(&serde_json::json!({
            "content": result.content,
            "is_error": result.is_error.unwrap_or(false),
        }))
        .unwrap()
    );
}
