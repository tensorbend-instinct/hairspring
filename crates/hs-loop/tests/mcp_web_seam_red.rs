//! RED (Eric 2026-09-07: "give the harness first-class web search + online
//! research tooling via the MCP plugin system"): the MCP tool seam must
//! exist on the TB runner and the REPL, not only on hs-swe-run.
//!
//! Validated live before wiring (box, 2026-09-07): hs-plugin-mcpcall
//! driving uvx duckduckgo-mcp-server (no key) - mcp.ddg.search returned
//! real results for "current stable Rust version", `mcp.ddg.fetch_content`
//! returned clean text of <https://releases.rs>/ with pagination.
//!
//! SWE-bench network lock (`HS_SWE_NET=off`) is untouched: nothing here
//! changes hs-swe-run or its builtin surface.

use hs_loop::repl::run_one_shot;
use std::process::Command;

const TB_DRIVER: &str = env!("CARGO_BIN_EXE_hs-tb-run");
const FIXTURE: &str = env!("CARGO_BIN_EXE_hs-mcp-fixture");
const ANSWER: &str = env!("CARGO_BIN_EXE_hs-plugin-answer");
const CHECKER: &str = env!("CARGO_BIN_EXE_hs-plugin-liechecker");
const SCRIPTED: &str = env!("CARGO_BIN_EXE_hs-plugin-scripted");
const MCPCALL: &str = env!("CARGO_BIN_EXE_hs-plugin-mcpcall");

// HS_MCP_SERVERS + HS_SEQMODEL_SCRIPT are process-global: serialize.
static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn servers_toml(dir: &std::path::Path) -> std::path::PathBuf {
    let p = dir.join("mcp_servers.toml");
    std::fs::write(
        &p,
        format!(
            "[[mcp_servers]]\nname = \"fixture\"\ncommand = [\"{FIXTURE}\"]\nallowed_roots = [\"{}\"]\n",
            dir.display()
        ),
    )
    .unwrap();
    p
}

// T1: the TB runner honors HS_MCP_SERVERS - discovered tools land in the
// generated kernel config AND the native tool schema (tools.json), exactly
// like hs-swe-run's seam (driver_mission_calls_mcp_tool).
#[test]
fn t1_tb_runner_registers_mcp_tools() {
    let _guard = ENV_LOCK.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let dir = tempfile::tempdir().unwrap();
    let run_dir = dir.path().join("run");
    let ws = run_dir.join("ws");
    std::fs::create_dir_all(&ws).unwrap();
    std::fs::write(ws.join("code.txt"), "broken\n").unwrap();
    std::fs::write(ws.join("check.sh"), "#!/bin/sh\ngrep -q '^fixed$' code.txt\n").unwrap();
    let inst = dir.path().join("instruction.md");
    std::fs::write(&inst, "Make code.txt contain the word fixed.\n").unwrap();
    let servers = servers_toml(dir.path());

    let out = Command::new(TB_DRIVER)
        .args([
            "--instruction", &inst.display().to_string(),
            "--task-id", "fixture__tb-mcp",
            "--model", "swemodel",
            "--feedback", "on",
            "--budget-micros", "100000",
            "--max-steps", "8",
            "--run-dir", &run_dir.display().to_string(),
            "--workdir", &ws.display().to_string(),
        ])
        .env("HS_MCP_SERVERS", &servers)
        .output()
        .unwrap();
    assert!(out.status.success(), "driver: {}", String::from_utf8_lossy(&out.stderr));

    let config = std::fs::read_to_string(run_dir.join("hairspring.toml")).unwrap();
    assert!(
        config.contains("mcp.fixture.echo"),
        "mcp tool registered in the generated kernel config:\n{config}"
    );
    let tools: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(run_dir.join("tools.json")).unwrap(),
    )
    .unwrap();
    let names: Vec<&str> = tools
        .as_array()
        .expect("tools.json is an array")
        .iter()
        .filter_map(|t| t["function"]["name"].as_str())
        .collect();
    let echo = tools
        .as_array()
        .unwrap()
        .iter()
        .find(|t| t["function"]["name"] == "mcp.fixture.echo")
        .unwrap_or_else(|| panic!("mcp.fixture.echo in the native schema: {names:?}"));
    assert!(
        echo["function"]["parameters"]["properties"]["text"].is_object(),
        "the server's input schema rides along: {echo}"
    );
    // the mission prompt must ADVERTISE the mcp tool (name + arg schema) -
    // the TB model path is free-form, the prompt is the only tool doc
    let mprompt = std::fs::read_to_string(run_dir.join("mission_prompt.txt")).unwrap();
    assert!(
        mprompt.contains("mcp.fixture.echo"),
        "mission prompt advertises the mcp tool:\n{mprompt}"
    );
    // and the mission itself still ran its normal blind flow
    let result: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(run_dir.join("result.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(result["passed"], true, "mission passes on its own checks: {result}");
}

// T2: the REPL honors HS_MCP_SERVERS - the discovered tool is registered in
// the session's kernel (callable through the real loop) and the goal prompt
// advertises it, so a real model can find it.
#[test]
fn t2_repl_mcp_tool_callable_and_advertised() {
    let _guard = ENV_LOCK.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let dir = tempfile::tempdir().unwrap();
    let log = tempfile::tempdir().unwrap();
    let servers = servers_toml(dir.path());
    let config = dir.path().join("hairspring.toml");
    std::fs::write(
        &config,
        format!(
            r#"
[[tools]]
name = "answer.write"
command = ["{ANSWER}"]
subjects = ["*"]

[[tools]]
name = "checker.run"
command = ["{CHECKER}"]
subjects = ["*"]

[[models]]
name = "scripted"
command = ["{SCRIPTED}"]
default = true
"#
        ),
    )
    .unwrap();
    let script = dir.path().join("script.jsonl");
    std::fs::write(
        &script,
        "{\"tool\":\"mcp.fixture.echo\",\"args\":{\"text\":\"hello-via-mcp\"}}\n".to_string()
            + &format!(
                "{{\"tool\":\"answer.write\",\"args\":{{\"path\":\"{}\",\"content\":\"REPL-MCP-TOKEN\"}}}}",
                log.path().join("work").join(hs_loop::repl::goal_slug("echo test")).join("answer.txt").display()
            ),
    )
    .unwrap();
    unsafe {
        std::env::set_var("HS_SEQMODEL_SCRIPT", &script);
        std::env::set_var("HS_MCP_SERVERS", &servers);
        // in-process discovery (REPL is a library): the bridge binary is not
        // a sibling of the test exe, so point at it explicitly.
        std::env::set_var("HS_MCP_BRIDGE_BIN", MCPCALL);
    }
    let r = run_one_shot(&config, log.path(), "echo test", false, 4).expect("one-shot run");
    unsafe {
        std::env::remove_var("HS_MCP_SERVERS");
        std::env::remove_var("HS_SEQMODEL_SCRIPT");
        std::env::remove_var("HS_MCP_BRIDGE_BIN");
    }
    assert!(r.passed, "mission passes: {r:?}");
    // REPL runs have no text ledger (hs-log is binary-segmented); the
    // behavioral proof is r.passed above: the mock only emits the needle
    // after calling mcp.fixture.echo. Config proof: the merged per-run
    // config on disk must carry the MCP tool stanza.
    let merged = std::fs::read_to_string(log.path().join("repl-hairspring.toml")).unwrap();
    assert!(
        merged.contains("mcp.fixture.echo"),
        "merged REPL config must register the MCP tool:\n{merged}"
    );
}
