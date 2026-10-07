//! REPL tool-env wiring: an hs-repl mission's term.exec must run in the
//! session's working directory with NO operator env exports.
//!
//! Live defect (2026-09-08): every term.exec in a live hs-repl run failed
//! with `spawn: No such file or directory`. Chain: hs-plugin-termexec
//! takes its working directory from `HS_TERM_WORKDIR` (default /app, which
//! does not exist on this host), and only hs-tb-run sets it. The
//! 2026-09-07 22:42 real-goal proof passed only because the operator
//! exported HS_TERM_WORKDIR=/tmp/repl-goal-app by hand - hs-repl itself
//! never wires the env. An intuitive REPL does not make the operator
//! learn internal plugin env vars.
//!
//! Contract: `ReplSession` wires the tool environment itself - term.exec
//! runs with cwd = the session dir (the REPL's working directory,
//! stable across missions), matching hs-tb-run's "every plugin the
//! kernel spawns inherits this env" rule.

use hs_core::EventKind;
use hs_loop::repl::ReplSession;

static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

const ANSWER: &str = env!("CARGO_BIN_EXE_hs-plugin-answer");
const TERMEXEC: &str = env!("CARGO_BIN_EXE_hs-plugin-termexec");
const SCRIPTED: &str = env!("CARGO_BIN_EXE_hs-plugin-scripted");

#[test]
fn repl_mission_term_exec_runs_in_session_dir() {
    let _guard = ENV_LOCK.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let dir = tempfile::tempdir().unwrap();
    let log = tempfile::tempdir().unwrap();
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
name = "term.exec"
command = ["{TERMEXEC}"]
subjects = ["*"]

[[models]]
name = "scripted"
command = ["{SCRIPTED}"]
default = true
"#
        ),
    )
    .unwrap();

    // The defect's precondition: the operator exported NOTHING.
    unsafe { std::env::remove_var("HS_TERM_WORKDIR") };

    let script = dir.path().join("s.jsonl");
    std::fs::write(
        &script,
        serde_json::json!({"tool":"term.exec","args":{"command":"pwd"}}).to_string(),
    )
    .unwrap();
    unsafe { std::env::set_var("HS_SEQMODEL_SCRIPT", &script) };

    let mut session = ReplSession::load(&config, log.path(), true, Some(1)).unwrap();
    session.run_goal("where do my shell commands run").unwrap();

    // The term.exec result must be the session dir, not a spawn error.
    let reader = hs_log::StreamReader::open(log.path(), session.stream_id()).unwrap();
    let results: Vec<String> = reader
        .events()
        .unwrap()
        .iter()
        .filter(|e| e.kind == EventKind::ToolCall)
        .filter_map(|e| {
            let bytes = reader.resolve_payload(e).ok()?;
            let v: serde_json::Value = serde_json::from_slice(&bytes).ok()?;
            if !v["error"].is_null() {
                return Some(format!("error: {}", v["error"]));
            }
            // term.exec's result is an object: {exit_code, timed_out,
            // stdout, stderr}; a spawn failure comes back as $error at
            // the plugin layer (surfaced as the kernel's error field).
            let r = &v["result"];
            if let Some(s) = r.as_str() {
                return Some(s.to_string());
            }
            if let Some(s) = r["stdout"].as_str() {
                return Some(format!("exit={} stdout={}", r["exit_code"], s));
            }
            Some(format!("unexpected result shape: {r}"))
        })
        .collect();
    assert_eq!(results.len(), 1, "one term.exec call: {results:?}");
    // D6: the session work area is <run>/work (harness state stays
    // outside the tool anchor).
    let want = log.path().join("work").canonicalize().unwrap();
    let expect = format!("exit=0 stdout={}", want.display());
    assert!(
        results[0].trim_end() == expect,
        "term.exec runs in the session dir - got: {:?} want: {:?}",
        results[0],
        expect
    );
}
