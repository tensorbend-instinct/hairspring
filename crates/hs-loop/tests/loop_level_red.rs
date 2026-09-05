//! Phase 1 RED (design D4 mission half + D6 budgets): when a plugin dies
//! for good, the mission ABORTS as harness_error with partial progress
//! booked (never burns remaining steps against a dead plugin, never crashes
//! the runner); and the loop checkpoints progress.json after every step so
//! an external wall-clock kill can book steps-so-far instead of a 0-step row.

use hs_loop::*;

const ANSWER: &str = env!("CARGO_BIN_EXE_hs-plugin-answer");
const CHECKER: &str = env!("CARGO_BIN_EXE_hs-plugin-checker");
const BENCHMODEL: &str = env!("CARGO_BIN_EXE_hs-plugin-benchmodel");
const SEQMODEL: &str = env!("CARGO_BIN_EXE_hs-seqmodel");
const FIXTURE: &str = env!("CARGO_BIN_EXE_hs-loopfix");

fn write(dir: &std::path::Path, name: &str, body: &str) -> std::path::PathBuf {
    let p = dir.join(name);
    std::fs::write(&p, body).unwrap();
    p
}

/// T3 (mission half): a permanently dead tool aborts the mission as
/// harness_error with steps-so-far - the loop does not feed the error back
/// and burn the remaining 24+ steps like run 17117 did.
#[test]
fn mission_aborts_as_harness_error_when_plugin_dies() {
    let dir = tempfile::tempdir().unwrap();
    let log = tempfile::tempdir().unwrap();
    let script = write(
        dir.path(),
        "script.jsonl",
        r#"{"tool":"zombie","args":{}}"#,
    );
    std::env::set_var("HS_SEQMODEL_SCRIPT", &script);
    let config = write(
        dir.path(),
        "hairspring.toml",
        &format!(
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
name = "zombie"
command = ["{FIXTURE}"]
subjects = ["*"]

[[models]]
name = "seqmodel"
command = ["{SEQMODEL}"]
default = true
"#
        ),
    );
    let kernel = hs_kernel::Kernel::load(&config).unwrap();
    let mut l = InnerLoop::new(kernel, log.path(), true, 10).unwrap();
    let r = l.run_mission("task-0").unwrap();
    assert_eq!(r.passed, false);
    assert_eq!(r.steps, 1, "abort on the failing step, not the cap: {:?}", r.steps);
    assert!(
        r.harness_error.is_some(),
        "mission must report harness_error, got: {r:?}"
    );
    let msg = r.harness_error.unwrap();
    assert!(msg.contains("zombie"), "names the dead plugin: {msg}");
    // the abort is booked into the stream for the trace
    let reader = hs_log::StreamReader::open(log.path(), r.stream_id).unwrap();
    let events = reader.events().unwrap();
    let booked = events.iter().any(|e| {
        e.kind == hs_core::EventKind::Feedback && {
            let b = reader.resolve_payload(e).unwrap_or_default();
            let s = String::from_utf8_lossy(&b);
            s.contains("harness_error") && s.contains("zombie")
        }
    });
    assert!(booked, "harness_error abort must be booked as a Feedback event");
}

/// T5a: the loop checkpoints progress.json after EVERY step (external
/// wall-clock kill must never produce a 0-step result for a run that worked).
#[test]
fn progress_json_checkpointed_per_step() {
    let dir = tempfile::tempdir().unwrap();
    let log = tempfile::tempdir().unwrap();
    let progress = log.path().join("progress.json");
    let config = write(
        dir.path(),
        "hairspring.toml",
        &format!(
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
name = "benchmodel"
command = ["{BENCHMODEL}"]
default = true
"#
        ),
    );
    let kernel = hs_kernel::Kernel::load(&config).unwrap();
    let mut l = InnerLoop::new(kernel, log.path(), true, 3).unwrap();
    l.set_progress_path(&progress);
    // task-18 is unrepairable: runs to the cap without passing
    let r = l.run_mission("task-18").unwrap();
    assert!(!r.passed);
    let v: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&progress).unwrap()).unwrap();
    assert_eq!(v["steps"].as_u64().unwrap(), 3, "checkpoint tracks steps");
    assert_eq!(v["model_calls"].as_u64().unwrap(), 3);
    assert_eq!(r.steps, 3);
}

/// T5b: booking a wall kill from the checkpoint yields the true step count,
/// never the 0-step row the old runner wrote on timeout.
#[test]
fn wall_kill_books_partial_from_checkpoint() {
    let dir = tempfile::tempdir().unwrap();
    let progress = dir.path().join("progress.json");
    std::fs::write(
        &progress,
        r#"{"steps": 22, "model_calls": 22, "cost_micros": 12345}"#,
    )
    .unwrap();
    let result = book_wall_kill(&progress, "conan-io__conan-17102", "kimi", true);
    assert_eq!(result["passed"], false);
    assert_eq!(result["steps"], 22, "true steps, not 0");
    assert_eq!(result["model_calls"], 22);
    assert_eq!(result["outcome"], "wall_killed");
    assert_eq!(result["instance_id"], "conan-io__conan-17102");
    assert_eq!(result["cost_micros"], 12345);
}
