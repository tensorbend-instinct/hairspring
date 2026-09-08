//! REPL PARITY GAP #2: mid-mission steering / interrupt.
//!
//! pi/omp-class REPLs let the operator talk to a RUNNING mission: a typed
//! line lands as a fresh user message before the next model call
//! (steering), and Esc/ctrl-c stops the mission cleanly with its artifacts
//! booked (interrupt). hs-repl today runs a mission to completion with no
//! operator channel at all - that is the red.
//!
//! Mechanism under test (deterministic, no threads): the operator channel
//! is a steering inbox FILE and an interrupt flag FILE the loop checks at
//! every step boundary. The interactive REPL writes the file when the user
//! types; headless runs can be steered from outside the process. The test
//! drives it through the loop's own tools: step 1 answer.write's the
//! steering line into the inbox, step 2's prompt must carry it verbatim as
//! a user message; step 1's prompt must not.

use hs_core::EventKind;
use hs_loop::*;

// HS_SEQMODEL_SCRIPT is process-global: serialize the scripted-model tests.
static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

const ANSWER: &str = env!("CARGO_BIN_EXE_hs-plugin-answer");
const CHECKER: &str = env!("CARGO_BIN_EXE_hs-plugin-checker");
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
    std::fs::write(
        &p,
        lines
            .iter()
            .map(|l| l.to_string())
            .collect::<Vec<_>>()
            .join("\n"),
    )
    .unwrap();
    p
}

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
            // the payload carries both the outbound messages AND the model's
            // completion; the completion legitimately contains the steering
            // line (it is the tool args that wrote the inbox). Prompt
            // assertions scan the messages half only.
            let v: serde_json::Value = serde_json::from_str(&body).unwrap();
            serde_json::to_string(&v["messages"]).unwrap()
        })
        .collect()
}

#[test]
fn steering_line_lands_in_next_step_prompt() {
    let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let dir = tempfile::tempdir().unwrap();
    let log = tempfile::tempdir().unwrap();
    let inbox = dir.path().join("steering.txt");
    let steer_line = "skip the docs, fix the parser first";
    let answer_path = log.path().join("work").join("task-0").join("answer.txt");
    let lines = vec![
        // step 1: the "operator" types - modeled by writing the inbox file
        serde_json::json!({"tool":"answer.write","args":{"path":inbox.display().to_string(),"content":steer_line}}),
        // steps 2-3: ordinary work
        serde_json::json!({"tool":"answer.write","args":{"path":answer_path.display().to_string(),"content":"working"}}),
        serde_json::json!({"tool":"answer.write","args":{"path":answer_path.display().to_string(),"content":"working more"}}),
    ];
    let script = write_script(dir.path(), &lines);
    unsafe { std::env::set_var("HS_SEQMODEL_SCRIPT", &script) };

    let mut l = rig(dir.path(), log.path(), 3);
    l.set_steering_inbox(&inbox);
    let stream = l.stream_id();
    let r = l.run_mission("task-0").unwrap();
    assert_eq!(r.steps, 3, "mission runs its steps: {r:?}");

    let prompts = model_call_prompts(log.path(), stream);
    assert!(prompts.len() >= 2, "at least two model calls: {prompts:?}");
    assert!(
        !prompts[0].contains(steer_line),
        "step-1 prompt predates the steering line"
    );
    assert!(
        prompts[1].contains(steer_line),
        "step-2 prompt carries the steering line verbatim as a user message: {}",
        &prompts[1][..prompts[1].len().min(400)]
    );
    // the inbox is drained, not re-read: the line must not duplicate into
    // every later prompt as an unread file would
    assert!(
        !inbox.exists() || std::fs::read_to_string(&inbox).unwrap().trim().is_empty(),
        "steering inbox drained after consumption"
    );
}

#[test]
fn interrupt_file_stops_mission_cleanly() {
    let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let dir = tempfile::tempdir().unwrap();
    let log = tempfile::tempdir().unwrap();
    let interrupt = dir.path().join("interrupt");
    let answer_path = log.path().join("work").join("task-0").join("answer.txt");
    let lines = vec![
        // step 1: the operator hits Esc - modeled by creating the flag file
        serde_json::json!({"tool":"answer.write","args":{"path":interrupt.display().to_string(),"content":""}}),
        // would run if the loop ignored the flag
        serde_json::json!({"tool":"answer.write","args":{"path":answer_path.display().to_string(),"content":"should never happen"}}),
    ];
    let script = write_script(dir.path(), &lines);
    unsafe { std::env::set_var("HS_SEQMODEL_SCRIPT", &script) };

    let mut l = rig(dir.path(), log.path(), 5);
    l.set_interrupt_file(&interrupt);
    let r = l.run_mission("task-0").unwrap();
    assert_eq!(
        r.outcome, "interrupted",
        "interrupt resolves as its own outcome, not a pass or an error: {r:?}"
    );
    assert_eq!(r.steps, 1, "loop stopped at the step boundary: {r:?}");
    assert!(!r.passed, "an interrupted mission is not a pass");
    assert!(
        r.harness_error.is_none(),
        "interrupt is operator intent, not a harness failure: {r:?}"
    );
}
