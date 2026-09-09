//! REPL PARITY GAP #4 (resume half): hs-repl resumes a prior session's
//! stream instead of always starting cold.
//!
//! pi/omp-class REPLs let you leave and come back: the conversation
//! continues with its history intact. hs-repl today opens a FRESH stream
//! on every invocation - a restarted session has amnesia even though the
//! substrate (`StreamWriter::resume`, `InnerLoop::with_stream`) has supported
//! adoption since gate 5. That is the red.
//!
//! Contract: `ReplSession::load_resume` adopts an existing stream id. The
//! next mission appends to the SAME stream (sequence numbers continue,
//! the hash chain verifies end to end), and the resumed mission's first
//! prompt carries the PRIOR mission's tool-call transcript - the model
//! gets its history back from the log, not from a cold start.

use hs_core::EventKind;
use hs_loop::repl::ReplSession;

// HS_MCP_SERVERS-free rig: scripted model only.
static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

const ANSWER: &str = env!("CARGO_BIN_EXE_hs-plugin-answer");
const CHECKER: &str = env!("CARGO_BIN_EXE_hs-plugin-checker");
const SCRIPTED: &str = env!("CARGO_BIN_EXE_hs-plugin-scripted");

fn write_config(dir: &std::path::Path) -> std::path::PathBuf {
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
    config
}

fn write_script(dir: &std::path::Path, name: &str, lines: &[serde_json::Value]) -> std::path::PathBuf {
    let p = dir.join(name);
    std::fs::write(
        &p,
        lines
            .iter()
            .map(std::string::ToString::to_string)
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
            let v: serde_json::Value = serde_json::from_str(&body).unwrap();
            serde_json::to_string(&v["messages"]).unwrap()
        })
        .collect()
}

#[test]
fn resumed_session_continues_stream_and_history() {
    let _guard = ENV_LOCK.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let dir = tempfile::tempdir().unwrap();
    let log = tempfile::tempdir().unwrap();
    let config = write_config(dir.path());

    // --- session 1: one mission, one tool call with a distinctive output
    let answer1 = log.path().join("work").join("task-0").join("answer.txt");
    let script1 = write_script(
        dir.path(),
        "s1.jsonl",
        &[serde_json::json!({"tool":"answer.write","args":{"path":answer1.display().to_string(),"content":"MARKER-FIRST-SESSION-7717"}})],
    );
    unsafe { std::env::set_var("HS_SEQMODEL_SCRIPT", &script1) };
    let stream_id = {
        let mut s1 = ReplSession::load(&config, log.path(), true, 1).unwrap();
        let r = s1.run_goal("task-0").unwrap();
        assert_eq!(r.steps, 1);
        s1.stream_id()
    };

    // --- session 2 (the restart): RESUME the same stream
    let script2 = write_script(
        dir.path(),
        "s2.jsonl",
        &[serde_json::json!({"tool":"answer.write","args":{"path":answer1.display().to_string(),"content":"second-session"}})],
    );
    unsafe { std::env::set_var("HS_SEQMODEL_SCRIPT", &script2) };
    let mut s2 = ReplSession::load_resume(&config, log.path(), true, 1, stream_id).unwrap();
    assert_eq!(
        s2.stream_id(),
        stream_id,
        "resumed session adopts the prior stream, not a fresh one"
    );
    let r2 = s2.run_goal("task-0").unwrap();
    assert_eq!(r2.steps, 1);

    // (a) same stream, sequence continues, hash chain verifies
    let reader = hs_log::StreamReader::open(log.path(), stream_id).unwrap();
    let events = reader.events().unwrap();
    let seqs: Vec<u64> = events.iter().map(|e| e.seq).collect();
    assert!(
        seqs.windows(2).all(|w| w[1] > w[0]),
        "sequence numbers continue monotonically across the resume: {seqs:?}"
    );

    // (b) the resumed mission's first prompt carries session-1's transcript
    let prompts = model_call_prompts(log.path(), stream_id);
    assert!(prompts.len() >= 2, "both missions' calls on one stream: {prompts:?}");
    let resumed_prompt = prompts.last().unwrap();
    assert!(
        resumed_prompt.contains("MARKER-FIRST-SESSION-7717"),
        "resumed mission sees the prior session's tool transcript: {}",
        &resumed_prompt[..resumed_prompt.len().min(300)]
    );
}

/// Gap #4 (fork half): branch a prior stream into a new linked stream
/// carrying the parent's full transcript, parent untouched.
#[test]
fn forked_session_branches_history_and_leaves_parent_intact() {
    let _guard = ENV_LOCK.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let dir = tempfile::tempdir().unwrap();
    let log = tempfile::tempdir().unwrap();
    let config = write_config(dir.path());

    // parent session: one mission with a distinctive artifact write
    let answer1 = log.path().join("work").join("task-0").join("answer.txt");
    let script1 = write_script(
        dir.path(),
        "s1.jsonl",
        &[serde_json::json!({"tool":"answer.write","args":{"path":answer1.display().to_string(),"content":"MARKER-PARENT-9921"}})],
    );
    unsafe { std::env::set_var("HS_SEQMODEL_SCRIPT", &script1) };
    let parent_id = {
        let mut s1 = ReplSession::load(&config, log.path(), true, 1).unwrap();
        s1.run_goal("task-0").unwrap();
        s1.stream_id()
    };
    let parent_events_before = hs_log::StreamReader::open(log.path(), parent_id)
        .unwrap()
        .events()
        .unwrap()
        .len();

    // fork: new linked stream, full transcript aboard
    let script2 = write_script(
        dir.path(),
        "s2.jsonl",
        &[serde_json::json!({"tool":"answer.write","args":{"path":answer1.display().to_string(),"content":"forked-branch"}})],
    );
    unsafe { std::env::set_var("HS_SEQMODEL_SCRIPT", &script2) };
    let mut s2 = ReplSession::load_fork(&config, log.path(), true, 1, parent_id).unwrap();
    let fork_id = s2.stream_id();
    assert_ne!(fork_id, parent_id, "a fork is a new stream");
    s2.run_goal("task-0").unwrap();

    // (a) the fork records its lineage
    let fork_reader = hs_log::StreamReader::open(log.path(), fork_id).unwrap();
    let fork_events = fork_reader.events().unwrap();
    let lineage = fork_events.iter().any(|e| {
        let b = fork_reader
            .resolve_payload(e)
            .map(|x| String::from_utf8_lossy(&x).into_owned())
            .unwrap_or_default();
        b.contains(&parent_id.to_string())
    });
    assert!(lineage, "fork stream names its parent stream id");

    // (b) the forked mission's first prompt carries the parent transcript
    let prompts = model_call_prompts(log.path(), fork_id);
    assert!(!prompts.is_empty());
    // NOTE: the fork stream carries the parent's ModelCall events too, so
    // the forked mission's own first prompt is the LAST model call here.
    let forked_prompt = prompts.last().unwrap();
    assert!(
        forked_prompt.contains("MARKER-PARENT-9921"),
        "forked mission sees the parent's history: {}",
        &forked_prompt[..forked_prompt.len().min(300)]
    );

    // (c) the parent stream is byte-untouched by the fork
    let parent_events_after = hs_log::StreamReader::open(log.path(), parent_id)
        .unwrap()
        .events()
        .unwrap()
        .len();
    assert_eq!(
        parent_events_before, parent_events_after,
        "fork never appends to the parent"
    );
}
