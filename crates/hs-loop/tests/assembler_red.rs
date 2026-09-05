//! RED acceptance gates for Phase 2 (D1 context assembler + D2 execution
//! ledger), from the replication design doc's TDD plan. These measure
//! MISSION BEHAVIOR, not mechanism (Eric's law: the old compaction tests
//! measured buffer size; these measure what the model experiences).
//!
//! T1 no_context_wipe_50_steps: a 50-step mission whose transcript fits the
//!    200k-token budget emits ZERO ContextInject why=pressure events, and the
//!    final prompt still carries the FIRST tool result verbatim.
//! T2 dup_read_flagged: an identical (tool,args) repeat produces an explicit
//!    duplicate note naming the prior seq, visible in a later prompt.
//! T8 ledger_bounded: the ledger summary stays under 2k tokens (8000 chars)
//!    at step 50, 100, 200.

use hs_core::{EventKind, Payload};
use hs_loop::*;
use std::io::Write;
use std::process::{Command, Stdio};

// HS_SEQMODEL_SCRIPT is process-global: serialize the mission tests.
static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

const ANSWER: &str = env!("CARGO_BIN_EXE_hs-plugin-answer");
const CHECKER: &str = env!("CARGO_BIN_EXE_hs-plugin-checker");
const BIGREAD: &str = env!("CARGO_BIN_EXE_hs-plugin-bigread");
const SCRIPTED: &str = env!("CARGO_BIN_EXE_hs-plugin-scripted");

fn rig(dir: &std::path::Path, log: &std::path::Path, max_steps: u32) -> InnerLoop {
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
command = ["{CHECKER}"]
subjects = ["*"]

[[tools]]
name = "bigread.read"
command = ["{BIGREAD}"]
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

fn stream_events(log: &std::path::Path, stream: uuid::Uuid) -> Vec<(EventKind, String)> {
    let reader = hs_log::StreamReader::open(log, stream).unwrap();
    reader
        .events()
        .unwrap()
        .iter()
        .map(|e| {
            let body = reader.resolve_payload(e)
                .map(|b| String::from_utf8_lossy(&b).into_owned())
                .unwrap_or_default();
            (e.kind, body)
        })
        .collect()
}

#[test]
fn t1_no_context_wipe_50_steps() {
    let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let dir = tempfile::tempdir().unwrap();
    let log = tempfile::tempdir().unwrap();
    // 49 big reads (~8KB each => ~400KB transcript, far over the old 60KB
    // char window, comfortably under a 200k-token budget), then blind answers
    let mut lines: Vec<serde_json::Value> = (1..=49)
        .map(|i| serde_json::json!({"tool":"bigread.read","args":{"page":i,"size":8000}}))
        .collect();
    let answer_path = log.path().join("work").join("task-0").join("answer.txt");
    lines.push(serde_json::json!({"tool":"answer.write","args":{"path":answer_path.display().to_string(),"content":"blind"}}));
    let script = write_script(dir.path(), &lines);
    unsafe { std::env::set_var("HS_SEQMODEL_SCRIPT", &script) };

    let mut l = rig(dir.path(), log.path(), 50);
    let stream = l.stream_id();
    let r = l.run_mission("task-0").unwrap();
    assert_eq!(r.steps, 50, "mission runs all 50 steps: {r:?}");

    let events = stream_events(log.path(), stream);
    let pressure: Vec<&String> = events.iter()
        .filter(|(k, b)| *k == EventKind::ContextInject && b.contains("why=pressure"))
        .map(|(_, b)| b)
        .collect();
    assert!(pressure.is_empty(), "zero context wipes under budget, got {}: {pressure:?}", pressure.len());

    // transcript honesty: the FINAL prompt still carries the first read
    let last_prompt = events.iter().rev()
        .find(|(k, _)| *k == EventKind::ModelCall)
        .map(|(_, b)| b.clone())
        .expect("a ModelCall event");
    assert!(last_prompt.contains("PAGE-1 "), "step-50 prompt still holds the first tool result verbatim");
}

#[test]
fn t2_dup_read_flagged_with_prior_seq() {
    let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let dir = tempfile::tempdir().unwrap();
    let log = tempfile::tempdir().unwrap();
    let answer_path = log.path().join("work").join("task-0").join("answer.txt");
    let read = serde_json::json!({"tool":"bigread.read","args":{"page":1,"size":8000}});
    let lines = vec![
        read.clone(),
        read.clone(), // exact duplicate: same tool, same args
        serde_json::json!({"tool":"bigread.read","args":{"page":2,"size":8000}}),
        serde_json::json!({"tool":"answer.write","args":{"path":answer_path.display().to_string(),"content":"blind"}}),
    ];
    let script = write_script(dir.path(), &lines);
    unsafe { std::env::set_var("HS_SEQMODEL_SCRIPT", &script) };

    let mut l = rig(dir.path(), log.path(), 6);
    let stream = l.stream_id();
    let _ = l.run_mission("task-0").unwrap();

    let events = stream_events(log.path(), stream);
    // a Feedback event flags the duplicate, naming the earlier seq
    let flag = events.iter()
        .find(|(k, b)| *k == EventKind::Feedback && b.contains("duplicate") && b.contains("seq"))
        .map(|(_, b)| b.clone())
        .expect("a Feedback event flagging the duplicate call with its prior seq");
    // and the model actually SEES the flag in a later prompt
    let later_prompt = events.iter()
        .skip_while(|(k, b)| !(*k == EventKind::Feedback && b.contains("duplicate")))
        .find(|(k, _)| *k == EventKind::ModelCall)
        .map(|(_, b)| b.clone())
        .expect("a ModelCall after the dup flag");
    assert!(later_prompt.contains("duplicate"), "dup flag reaches the model: {flag}");
    assert!(later_prompt.contains("already "), "names the prior call: {flag}");
}

#[test]
fn t8_ledger_summary_bounded_at_200_steps() {
    let mut ledger = ledger::Ledger::default();
    for seq in 1..=200u64 {
        let path = format!("src/file_{}.rs", seq % 37);
        ledger.apply_tool_call(seq, "repo.read",
            &serde_json::json!({"path": path, "start_line": (seq % 9) * 100 + 1, "max_lines": 100}),
            &serde_json::json!({"content": "x", "total_lines": 1000}));
        if seq % 5 == 0 {
            ledger.apply_tool_call(seq, "edit.apply",
                &serde_json::json!({"diff": "..."}),
                &serde_json::json!({"applied": true, "files_changed": [path]}));
        }
        if seq % 7 == 0 {
            ledger.apply_tool_call(seq, "repo.exec",
                &serde_json::json!({"command": format!("cargo test --test t{}", seq % 11)}),
                &serde_json::json!({"applied": true, "exit_code": (seq % 3 == 0) as i32}));
        }
        for probe in [50, 100, 200] {
            if seq == probe {
                let s = ledger.summary();
                assert!(s.len() <= ledger::LEDGER_SUMMARY_CHARS,
                    "summary bounded at step {probe}: {} chars > {}", s.len(), ledger::LEDGER_SUMMARY_CHARS);
            }
        }
    }
}

/// D1: when compression DOES fire, the oldest verbatim events are distilled
/// by the model into the Codex four-element handoff contract (progress and
/// decisions / constraints and preferences / next steps / critical data) -
/// never a bare tally. The summary links the source event range and the
/// distillation call is booked to the log with its cost.
#[test]
fn d1_handoff_summary_four_elements() {
    let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let dir = tempfile::tempdir().unwrap();
    let log = tempfile::tempdir().unwrap();
    let mut lines: Vec<serde_json::Value> = (1..=8)
        .map(|i| serde_json::json!({"tool":"bigread.read","args":{"page":i,"size":8000}}))
        .collect();
    let answer_path = log.path().join("work").join("task-0").join("answer.txt");
    lines.push(serde_json::json!({"tool":"answer.write","args":{"path":answer_path.display().to_string(),"content":"blind"}}));
    let script = write_script(dir.path(), &lines);
    unsafe { std::env::set_var("HS_SEQMODEL_SCRIPT", &script) };

    let mut l = rig(dir.path(), log.path(), 10);
    // small budget: compression must fire
    l.set_context_budget_tokens(16_000);
    let stream = l.stream_id();
    let r = l.run_mission("task-0").unwrap();
    assert_eq!(r.steps, 10, "{r:?}");

    let events = stream_events(log.path(), stream);
    // the distilled summary reaches later prompts with all four elements
    let last_prompt = events.iter().rev()
        .find(|(k, _)| *k == EventKind::ModelCall)
        .map(|(_, b)| b.clone())
        .expect("a ModelCall event");
    for element in ["PROGRESS AND DECISIONS", "CONSTRAINTS AND PREFERENCES", "NEXT STEPS", "CRITICAL DATA"] {
        assert!(last_prompt.contains(element), "handoff carries {element}");
    }
    assert!(last_prompt.contains("seq"), "summary links the source event range");
    // the distillation call itself is booked, with cost, marked why=distill
    let distill = events.iter()
        .find(|(k, b)| *k == EventKind::ModelCall && b.contains("\"why\":\"distill\""))
        .map(|(_, b)| b.clone())
        .expect("a ModelCall event with why=distill");
    assert!(distill.contains("cost_usd_micros"), "distillation cost booked: {distill}");
}

/// W2: the default context budget comes from the VERIFIED provider context
/// (Moonshot docs: kimi-k3 = 1M tokens), minus an output/reasoning reserve,
/// not the 200k estimate; unknown models fall back conservatively.
#[test]
fn w2_budget_from_verified_context() {
    assert_eq!(default_budget_for_model("kimi-k3"), 983_040, "1M context - 64k reserve");
    assert_eq!(DEFAULT_CONTEXT_BUDGET_TOKENS, default_budget_for_model("kimi-k3"));
    assert_eq!(default_budget_for_model("unknown-model"), 200_000, "conservative fallback");
}
