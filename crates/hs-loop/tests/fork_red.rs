//! Async delegation (Eric ruling 2026-09-08: "async delegation, proper
//! delegation event paradigm, atomic safe, best practices"). The #5
//! shape was SYNCHRONOUS: agent.spawn blocked the parent until the
//! child finished, so SubAgentSpawned/Finished fired back-to-back
//! post-hoc and the panel could never show a child mid-run.
//!
//! Contract (D1-D5, approved by Eric via Main):
//! - agent.spawn returns IMMEDIATELY (status running); children run
//!   CONCURRENTLY on plugin-side threads.
//! - The loop mints `child_stream_id` and books Spawn BEFORE the call
//!   (atomic: a crash anywhere leaves consistent provenance).
//! - Outcomes arrive via `agent.spawn_poll` at step boundaries:
//!   `SubAgentFinished` + cost fold + an ungated delegation update.
//! - A passing close JOINS running children (books stay honest).
//! - Depth guard + a concurrency cap bound the tree.

use hs_loop::repl::load_session;
use hs_loop::uipaint::UiEvent;
use std::sync::{Arc, Mutex};
use std::time::Instant;

/// `HS_SEQMODEL_SCRIPT` + `HS_SWARM`_* are process-global: the mission
/// tests in this binary serialize on this lock.
static SERIAL: Mutex<()> = Mutex::new(());

fn write_fixture(dir: &std::path::Path) {
    std::fs::create_dir_all(dir).unwrap();
    let toml = concat!(r#"
[[tools]]
name = "answer.submit"
command = [""#, env!("CARGO_BIN_EXE_hs-plugin-answersubmit"), r#""]
subjects = ["*"]

[[tools]]
name = "checker.run"
command = [""#, env!("CARGO_BIN_EXE_hs-plugin-liechecker"), r#""]
subjects = ["*"]

[[tools]]
name = "agent.spawn"
command = [""#, env!("CARGO_MANIFEST_DIR"), "/../../target/debug/hs-plugin-swarm", r#""]
subjects = ["*"]

[[tools]]
name = "agent.spawn_poll"
command = [""#, env!("CARGO_MANIFEST_DIR"), "/../../target/debug/hs-plugin-swarm", r#"", "--as", "agent.spawn_poll"]
subjects = ["*"]

[[models]]
name = "scripted-parent"
command = ["/bin/sh", "-c", "HS_SCRIPTED_NAME=scripted-parent HS_SEQMODEL_DELAY_MS=2000 exec "#, env!("CARGO_BIN_EXE_hs-plugin-scripted"), r#""]
default = true
subjects = ["*"]

[[models]]
name = "scripted-fast"
command = ["/bin/sh", "-c", "HS_SCRIPTED_NAME=scripted-fast HS_SEQMODEL_DELAY_MS=300 exec "#, env!("CARGO_BIN_EXE_hs-plugin-scripted"), r#""]
subjects = ["*"]

[[models]]
name = "scripted-slow"
command = ["/bin/sh", "-c", "HS_SCRIPTED_NAME=scripted-slow HS_SEQMODEL_DELAY_MS=2000 exec "#, env!("CARGO_BIN_EXE_hs-plugin-scripted"), r#""]
subjects = ["*"]
"#);
    std::fs::write(dir.join("hairspring.toml"), toml).unwrap();
    // Parent (2s/call pacing): spawn slow, spawn fast, one poll on a
    // bogus id that just buys a step, then prompt-aware submit.
    // Children replay from line 1 under their OWN name: two spawn
    // attempts (depth-refused), the bogus poll (unknown id), then
    // prompt-aware submit - 4 calls each.
    // Timeline: slow spawned @2s, done ~10s (4x2s). Fast spawned @4s,
    // done ~5.2s (4x0.3s). Step-3-boundary poll @6s sees the fast
    // finish -> step-4 prompt carries DELEGATION UPDATES. Parent's
    // submit @8s JOINS the slow child -> wall ~10s (serial ~17s).
    std::fs::write(
        dir.join("script.jsonl"),
        "{\"tool\":\"agent.fork\",\"args\":{\"mission\":\"forked child\",\"model\":\"scripted-fast\"}}\n\
         {\"tool\":\"agent.spawn_poll\",\"args\":{\"child_stream_id\":\"00000000-0000-0000-0000-000000000000\"}}\n",
    )
    .unwrap();
}


#[test]
fn f1_unit_fork_mission_carries_context() {
    let m = hs_loop::fork_mission("do X", "reads: a.rs");
    assert!(m.starts_with("do X") && m.contains("FORKED CONTEXT") && m.contains("reads: a.rs"), "{m}");
    assert_eq!(hs_loop::fork_mission("do X", "  "), "do X");
}

#[test]
fn f2_agent_fork_spawns_a_booked_child() {
    let _serial = SERIAL.lock().unwrap();
    let dir = std::env::temp_dir().join("swarm-fork-f2");
    let _ = std::fs::remove_dir_all(&dir);
    write_fixture(&dir);
    unsafe { std::env::remove_var("HS_SWARM_DEPTH") };
    unsafe { std::env::set_var("HS_ANSWER_RAW", "1") };
    unsafe { std::env::set_var("HS_SCRIPTED_PROMPT_AWARE", "1") };
    unsafe { std::env::set_var("HS_SEQMODEL_SCRIPT", dir.join("script.jsonl")) };
    unsafe { std::env::set_var("HS_SWARM_LOG_ROOT", dir.join("run")) };
    unsafe { std::env::set_var("HS_SWARM_CONFIG", dir.join("hairspring.toml")) };
    let events: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
    let cap = events.clone();
    let mut s = load_session(&dir.join("hairspring.toml"), &dir.join("run"), false, Some(12), None, None).unwrap();
    s.set_ui_sink(Box::new(move |ev: UiEvent| cap.lock().unwrap().push(format!("{ev:?}"))));
    let r = s.run_goal("fork one child").unwrap();
    assert!(r.passed, "{r:?}");
    let ev = events.lock().unwrap();
    assert_eq!(ev.iter().filter(|e| e.starts_with("SubAgentSpawned") && e.contains("forked child")).count(), 1, "{ev:?}");
    assert_eq!(ev.iter().filter(|e| e.starts_with("SubAgentFinished")).count(), 1, "{ev:?}");
    let _ = Instant::now();
}
