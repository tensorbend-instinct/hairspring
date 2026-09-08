//! RED (Eric 2026-09-07 parity build, gap #1): the REPL must deliver NATIVE
//! tool schemas to the model plugin, exactly like hs-tb-run/hs-swe-run do via
//! InnerLoop::set_tools - the tools=None free-form path cannot hold a real
//! model on the one-tool-call protocol (live proof 2026-09-07: REPL real-goal
//! mission, DeepSeek author, 1 tool call then 39 prose replies, nudged every
//! step, steps_exhausted; TB/SWE use SYSTEM_NATIVE + tool_choice:"required"
//! and hold protocol by construction).

use hs_loop::repl::ReplSession;

const SCRIPTED: &str = env!("CARGO_BIN_EXE_hs-plugin-scripted");
const ANSWER: &str = env!("CARGO_BIN_EXE_hs-plugin-answer");
const CHECKER: &str = env!("CARGO_BIN_EXE_hs-plugin-checker");
const FIXTURE: &str = env!("CARGO_BIN_EXE_hs-mcp-fixture");
const MCPCALL: &str = env!("CARGO_BIN_EXE_hs-plugin-mcpcall");

fn test_config(dir: &std::path::Path) -> std::path::PathBuf {
    let p = dir.join("hairspring.toml");
    std::fs::write(
        &p,
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
    p
}

// One sequential test: HS_MCP_SERVERS is process-global env, so parallel
// tests would race on it (live: t1 picked up t2's fixture toml mid-flight).
#[test]
fn repl_delivers_native_schemas_builtin_and_mcp() {
    // (a) without HS_MCP_SERVERS: builtin native schemas must be delivered
    let dir = tempfile::tempdir().unwrap();
    let log = tempfile::tempdir().unwrap();
    // the scripted model plugin EOFs its visibility probe without a script
    let script = dir.path().join("script.jsonl");
    std::fs::write(&script, "{\"tool\":\"answer.write\",\"args\":{\"text\":\"x\"}}\n").unwrap();
    unsafe {
        std::env::remove_var("HS_MCP_SERVERS");
        std::env::set_var("HS_SEQMODEL_SCRIPT", &script);
    }
    let session = ReplSession::load(&test_config(dir.path()), log.path(), false, 4)
        .expect("session load");
    let names = session.native_tool_names();
    assert!(
        names.iter().any(|n| n == "answer.submit"),
        "builtin answer.submit schema delivered: {names:?}"
    );
    assert!(
        names.iter().any(|n| n == "term.exec"),
        "builtin term.exec schema delivered: {names:?}"
    );
    drop(session);

    // (b) with HS_MCP_SERVERS: discovered MCP tools ride the same delivery
    let dir2 = tempfile::tempdir().unwrap();
    let log2 = tempfile::tempdir().unwrap();
    let servers = dir2.path().join("mcp_servers.toml");
    std::fs::write(
        &servers,
        format!(
            "[[mcp_servers]]\nname = \"fixture\"\ncommand = [\"{FIXTURE}\"]\nallowed_roots = [\"{}\"]\n",
            dir2.path().display()
        ),
    )
    .unwrap();
    unsafe {
        std::env::set_var("HS_MCP_SERVERS", &servers);
        std::env::set_var("HS_MCP_BRIDGE_BIN", MCPCALL);
    }
    let session2 = ReplSession::load(&test_config(dir2.path()), log2.path(), false, 4)
        .expect("session load");
    unsafe {
        std::env::remove_var("HS_MCP_SERVERS");
        std::env::remove_var("HS_MCP_BRIDGE_BIN");
        std::env::remove_var("HS_SEQMODEL_SCRIPT");
    }
    let names2 = session2.native_tool_names();
    assert!(
        names2.iter().any(|n| n == "mcp.fixture.echo"),
        "mcp tool in the native delivery: {names2:?}"
    );
    assert!(
        names2.iter().any(|n| n == "term.exec"),
        "builtins ride along too: {names2:?}"
    );
}
