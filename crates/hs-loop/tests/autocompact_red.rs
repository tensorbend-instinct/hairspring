//! REPL PARITY GAP #6: auto-compaction at the MODEL's window, not a
//! fixed ~1M-token default.
//!
//! The loop's assembler compacts when the assembled transcript crosses
//! context_budget_chars (gate 9f, proven at the InnerLoop level). But
//! the REPL leaves that budget at DEFAULT_CONTEXT_BUDGET_TOKENS (983k
//! tokens) whatever the configured model is - DeepSeek's window is 64k
//! tokens, so a long hs-repl session dies on a provider context-overflow
//! 400 long before the compactor ever fires. pi/omp compact at the
//! model's real window; the session runs indefinitely.
//!
//! Contract: the REPL reads the default model's `context_tokens` from
//! the config and drives the loop's compaction budget from it (with
//! headroom for the reply), so window pressure triggers the assembler's
//! COMPACTED handoff and missions keep completing - no overflow, no
//! truncation, no amnesia.

use hs_core::EventKind;
use hs_loop::repl::ReplSession;

static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

const ANSWER: &str = env!("CARGO_BIN_EXE_hs-plugin-answer");
const CHECKER: &str = env!("CARGO_BIN_EXE_hs-plugin-checker");
const SCRIPTED: &str = env!("CARGO_BIN_EXE_hs-plugin-scripted");

fn model_call_prompts(log: &std::path::Path, stream: uuid::Uuid) -> Vec<String> {
    let reader = hs_log::StreamReader::open(log, stream).unwrap();
    reader
        .events()
        .unwrap()
        .iter()
        .filter(|e| e.kind == EventKind::ModelCall)
        .map(|e| {
            let body = reader
                .resolve_payload(e)
                .map(|b| String::from_utf8_lossy(&b).into_owned())
                .unwrap_or_default();
            let v: serde_json::Value = serde_json::from_str(&body).unwrap();
            serde_json::to_string(&v["messages"]).unwrap()
        })
        .collect()
}

#[test]
fn repl_compacts_at_the_configured_models_window() {
    let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let dir = tempfile::tempdir().unwrap();
    let log = tempfile::tempdir().unwrap();
    // A model with a small declared window: 1024 tokens = 4096 chars.
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
context_tokens = 1024
"#
        ),
    )
    .unwrap();

    // Mission 1: one tool call with ~6k chars of transcript (over the
    // 4096-char budget once replayed).
    let big = "Z".repeat(6000);
    let a1 = log.path().join("work/task-0/answer.txt");
    let s1 = dir.path().join("s1.jsonl");
    std::fs::write(
        &s1,
        serde_json::json!({"tool":"answer.write","args":{"path":a1.display().to_string(),"content":big}})
            .to_string(),
    )
    .unwrap();
    unsafe { std::env::set_var("HS_SEQMODEL_SCRIPT", &s1) };

    let mut session = ReplSession::load(&config, log.path(), true, 1).unwrap();
    session.run_goal("task-0").unwrap();

    // Mission 2: its first prompt must show the compactor fired - the
    // 6k exchange cannot fit verbatim in a 1024-token window.
    let a2 = log.path().join("work/task-1/answer.txt");
    let s2 = dir.path().join("s2.jsonl");
    std::fs::write(
        &s2,
        serde_json::json!({"tool":"answer.write","args":{"path":a2.display().to_string(),"content":"done"}})
            .to_string(),
    )
    .unwrap();
    unsafe { std::env::set_var("HS_SEQMODEL_SCRIPT", &s2) };
    session.run_goal("task-1").unwrap();

    let prompts = model_call_prompts(log.path(), session.stream_id());
    assert!(prompts.len() >= 2, "both missions called the model: {prompts:?}");
    let m2_prompt = prompts.last().unwrap();
    assert!(
        m2_prompt.contains("COMPACTED"),
        "window pressure auto-compacts the prior exchange into a handoff: {}",
        &m2_prompt[..m2_prompt.len().min(400)]
    );
    assert!(
        m2_prompt.len() < 6000 + 4096,
        "the over-window exchange is NOT replayed verbatim: {} chars",
        m2_prompt.len()
    );
}
