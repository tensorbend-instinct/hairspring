//! RED acceptance gates for Phase 3 (D3 typed memory plane + D6 goal
//! evaluator), from the design doc's TDD plan.
//!
//! T6 memory_roundtrip: mission A's extracted memory record is retrieved
//!    into mission B's context with its source_seqs intact.
//! T7 goal_evaluator_stops_green: a mission stops when acceptance is
//!    verifiably green (patch applies + F2P passes in the sandbox); it
//!    REFUSES to stop on red, even when the checker plugin says pass.

use hs_core::{EventKind, Payload};
use hs_loop::*;
use std::process::Command;

static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

const ANSWER: &str = env!("CARGO_BIN_EXE_hs-plugin-answer");
const CHECKER: &str = env!("CARGO_BIN_EXE_hs-plugin-checker");
const LIECHECKER: &str = env!("CARGO_BIN_EXE_hs-plugin-liechecker");
const SCRIPTED: &str = env!("CARGO_BIN_EXE_hs-plugin-scripted");

fn rig(dir: &std::path::Path, log: &std::path::Path, checker: &str, max_steps: u32) -> InnerLoop {
    let config = dir.join("hairspring.toml");
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
command = ["{checker}"]
subjects = ["*"]

[[models]]
name = "scripted"
command = ["{SCRIPTED}"]
default = true
"#
        ),
    )
    .unwrap();
    let kernel = hs_kernel::Kernel::load(&config).unwrap();
    InnerLoop::new(kernel, log, true, max_steps).unwrap()
}

fn write_script(dir: &std::path::Path, lines: &[serde_json::Value]) -> std::path::PathBuf {
    let p = dir.join("script.jsonl");
    std::fs::write(&p, lines.iter().map(|l| l.to_string()).collect::<Vec<_>>().join("\n")).unwrap();
    p
}

fn model_prompts(log: &std::path::Path, stream: uuid::Uuid) -> Vec<String> {
    let reader = hs_log::StreamReader::open(log, stream).unwrap();
    reader
        .events()
        .unwrap()
        .iter()
        .filter(|e| e.kind == EventKind::ModelCall)
        .filter_map(|e| reader.resolve_payload(e).ok())
        .map(|b| String::from_utf8_lossy(&b).into_owned())
        .collect()
}

#[test]
fn t6_memory_roundtrip_across_missions() {
    let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    // --- mission A: passes in one step ---
    let dir_a = tempfile::tempdir().unwrap();
    let log_a = tempfile::tempdir().unwrap();
    let answer_a = log_a.path().join("work").join("task-0").join("answer.txt");
    let script = write_script(dir_a.path(), &[serde_json::json!(
        {"tool":"answer.write","args":{"path":answer_a.display().to_string(),"content":"TOKEN-0-SECRET"}})]);
    unsafe { std::env::set_var("HS_SEQMODEL_SCRIPT", &script) };
    let mut la = rig(dir_a.path(), log_a.path(), CHECKER, 4);
    let stream_a = la.stream_id();
    let ra = la.run_mission("task-0").unwrap();
    assert!(ra.passed, "mission A passes: {ra:?}");

    // --- post-mission extraction (D3): trajectory -> typed records ---
    let records = hs_memory::extract::extract_stream(log_a.path(), stream_a, "task-0", "operator");
    assert!(!records.is_empty(), "extraction yields records");
    assert!(records.iter().all(|r| !r.source_seqs.is_empty()), "provenance mandatory: {records:?}");
    assert!(records.iter().any(|r| r.kind == "episodic"), "an episodic record: {records:?}");

    let db = tempfile::tempdir().unwrap().keep().join("memory.db");
    let store = hs_memory::sqlite::SqliteMemoryStore::open(&db).unwrap();
    for r in records {
        store.put(r).unwrap();
    }

    // --- mission B: the assembler retrieves mission A's record ---
    let dir_b = tempfile::tempdir().unwrap();
    let log_b = tempfile::tempdir().unwrap();
    let answer_b = log_b.path().join("work").join("task-0").join("answer.txt");
    let script = write_script(dir_b.path(), &[serde_json::json!(
        {"tool":"answer.write","args":{"path":answer_b.display().to_string(),"content":"blind"}})]);
    unsafe { std::env::set_var("HS_SEQMODEL_SCRIPT", &script) };
    let mut lb = rig(dir_b.path(), log_b.path(), CHECKER, 2);
    lb.set_memory_db(&db);
    let stream_b = lb.stream_id();
    let _ = lb.run_mission("task-0").unwrap();

    let prompts = model_prompts(log_b.path(), stream_b);
    let p = prompts.last().expect("a prompt in mission B");
    assert!(p.contains("MEMORY (earlier missions)"), "memory block assembled: {p}");
    assert!(p.contains("task-0"), "mission A's record content retrieved: {p}");
    // provenance rides along: the seq refs from mission A's log
    let first = store.top_k("operator", 5).unwrap().into_iter().next().unwrap();
    let seqref = format!("seqs:{}", first.source_seqs.iter().map(|s| s.to_string()).collect::<Vec<_>>().join(","));
    assert!(p.contains(&seqref), "source_seqs intact in the prompt: {seqref} in {p}");
}

fn mk_ws(dir: &std::path::Path) -> std::path::PathBuf {
    let ws = dir.join("ws");
    std::fs::create_dir_all(&ws).unwrap();
    std::fs::write(ws.join("code.txt"), "broken\n").unwrap();
    std::fs::write(ws.join("check.sh"), "#!/bin/sh\ngrep -q '^fixed$' code.txt\n").unwrap();
    let git = |args: &[&str]| {
        assert!(Command::new("git").args(args).current_dir(&ws)
            .env("GIT_AUTHOR_NAME", "t").env("GIT_AUTHOR_EMAIL", "t@t")
            .env("GIT_COMMITTER_NAME", "t").env("GIT_COMMITTER_EMAIL", "t@t")
            .status().unwrap().success());
    };
    git(&["init", "-q"]);
    git(&["add", "."]);
    git(&["commit", "-qm", "base"]);
    ws
}

#[test]
fn t7_goal_evaluator_stops_green_refuses_red() {
    let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let dir = tempfile::tempdir().unwrap();
    let ws = mk_ws(dir.path());
    let fix = "```diff\n--- a/code.txt\n+++ b/code.txt\n@@ -1 +1 @@\n-broken\n+fixed\n```";
    let wrong = "```diff\n--- a/code.txt\n+++ b/code.txt\n@@ -1 +1 @@\n-broken\n+still-broken\n```";

    // RED half: the checker plugin LIES (always pass); the acceptance
    // predicate (patch applies + F2P green in the sandbox) is red, so the
    // mission must NOT stop green.
    let log_r = tempfile::tempdir().unwrap();
    let answer_r = log_r.path().join("work").join("task-0").join("answer.txt");
    let script = write_script(dir.path(), &[serde_json::json!(
        {"tool":"answer.write","args":{"path":answer_r.display().to_string(),"content":wrong}})]);
    unsafe { std::env::set_var("HS_SEQMODEL_SCRIPT", &script) };
    let dir_r = tempfile::tempdir().unwrap();
    let mut lr = rig(dir_r.path(), log_r.path(), LIECHECKER, 3);
    lr.set_goal_evaluator(&ws, vec!["sh check.sh".to_string()]);
    let stream_r = lr.stream_id();
    let rr = lr.run_mission("task-0").unwrap();
    assert!(!rr.passed, "refuses to stop on red even when the checker lies: {rr:?}");
    assert_eq!(rr.steps, 3, "burns to the cap still working: {rr:?}");
    let reader = hs_log::StreamReader::open(log_r.path(), stream_r).unwrap();
    let goal_note = reader.events().unwrap().iter()
        .filter(|e| e.kind == EventKind::Feedback)
        .filter_map(|e| reader.resolve_payload(e).ok())
        .map(|b| String::from_utf8_lossy(&b).into_owned())
        .find(|b| b.contains("goal_evaluator"))
        .expect("a goal_evaluator Feedback event explaining the red stop-refusal");

    // GREEN half: the fixing patch stops the mission at step 1.
    let log_g = tempfile::tempdir().unwrap();
    let answer_g = log_g.path().join("work").join("task-0").join("answer.txt");
    let script = write_script(dir.path(), &[serde_json::json!(
        {"tool":"answer.write","args":{"path":answer_g.display().to_string(),"content":fix}})]);
    unsafe { std::env::set_var("HS_SEQMODEL_SCRIPT", &script) };
    let dir_g = tempfile::tempdir().unwrap();
    let mut lg = rig(dir_g.path(), log_g.path(), LIECHECKER, 3);
    lg.set_goal_evaluator(&ws, vec!["sh check.sh".to_string()]);
    let rg = lg.run_mission("task-0").unwrap();
    assert!(rg.passed, "stops when acceptance is verifiably green: {rg:?} (note: {goal_note})");
    assert_eq!(rg.steps, 1, "stops at the first green, not the cap: {rg:?}");
}
