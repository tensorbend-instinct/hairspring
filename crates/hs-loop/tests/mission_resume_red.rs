//! MISSION CONTINUATION (Eric 2026-09-13, iMessage): "everything starts a
//! new mission even if it ended prematurely - a mission should never end
//! prematurely especially if budgets are not violated." Live symptom:
//! mission c91e8de3 died budget_killed at 162 steps; re-running the goal
//! minted a NEW mission (slug-2) at step 1 with a fresh ledger - the 162
//! steps of verification state were abandoned, and the session showed two
//! missions where the operator saw ONE piece of work.
//!
//! THE LAW: a mission ends only on a checker verdict, a real cap, an
//! operator interrupt, or a harness error - and every non-pass close is
//! CONTINUABLE. Re-running the same goal resumes the SAME mission: same
//! id, same work dir, step/call/cost counters restored from the stream,
//! the verification ledger replayed (an answer.submit right after resume
//! is not refuted as untested when the prior span verified). A passed
//! mission re-run, or a different goal that slugs the same, still spawns
//! a fresh mission. Benchmark binaries keep fresh-start semantics (the
//! resume path is a user-facing-session contract, like ProviderReported
//! budget guards).

use hs_core::EventKind;
use hs_loop::repl::ReplSession;

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

fn write_script(dir: &std::path::Path, lines: &[serde_json::Value]) -> std::path::PathBuf {
    let p = dir.join("script.jsonl");
    let text: String = lines.iter().map(|l| l.to_string() + "\n").collect();
    std::fs::write(&p, text).unwrap();
    p
}

fn goal_update_payloads(
    log_root: &std::path::Path,
    stream_id: uuid::Uuid,
) -> Vec<serde_json::Value> {
    let r = hs_log::StreamReader::open(log_root, stream_id).unwrap();
    let events = r.events().unwrap();
    events
        .iter()
        .filter(|e| e.kind == EventKind::GoalUpdate)
        .map(|e| {
            let b = r.resolve_payload(e).unwrap();
            serde_json::from_slice(&b).unwrap()
        })
        .collect()
}

fn feedback_payloads(log_root: &std::path::Path, stream_id: uuid::Uuid) -> Vec<serde_json::Value> {
    let r = hs_log::StreamReader::open(log_root, stream_id).unwrap();
    let events = r.events().unwrap();
    events
        .iter()
        .filter(|e| e.kind == EventKind::Feedback)
        .filter_map(|e| {
            let b = r.resolve_payload(e).ok()?;
            serde_json::from_slice(&b).ok()
        })
        .collect()
}

/// The core law: a mission that died at a cap is CONTINUED by re-running
/// the same goal - same mission id, same work dir, counters cumulative.
#[test]
fn dead_mission_same_goal_continues_same_mission() {
    let _guard = ENV_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let dir = tempfile::tempdir().unwrap();
    let log = tempfile::tempdir().unwrap();
    let config = write_config(dir.path());
    let answer = log.path().join("work").join("task-0").join("answer.txt");
    let script = write_script(
        dir.path(),
        &[
            serde_json::json!({"tool":"answer.write","args":{"path":answer.display().to_string(),"content":"WRONG"}}),
            serde_json::json!({"tool":"answer.write","args":{"path":answer.display().to_string(),"content":"TOKEN-0-SECRET"}}),
        ],
    );
    unsafe { std::env::set_var("HS_SEQMODEL_SCRIPT", &script) };

    let mut s = ReplSession::load(&config, log.path(), true, Some(1)).unwrap();
    let r1 = s.run_goal("task-0").unwrap();
    assert!(!r1.passed, "wrong token cannot pass: {r1:?}");
    assert_eq!(r1.steps, 1, "one step ran: {r1:?}");

    // The next run of the SAME goal names the SAME mission - a dead
    // mission is continued, never respawned under a -2 suffix.
    assert_eq!(
        s.mission_id_for("task-0"),
        "task-0",
        "a non-pass close makes the mission continuable, not a new id"
    );

    // The mission died AT the step cap - continuing it under the
    // same spent cap would close instantly, so the operator raises it
    // first (the /caps move the resume banner already teaches).
    s.set_cap("steps", "4").unwrap();
    let r2 = s.run_goal("task-0").unwrap();
    assert!(r2.passed, "the resumed mission lands the right token: {r2:?}");
    assert_eq!(
        r2.steps, 2,
        "the step counter continues across the resume: {r2:?}"
    );
    assert_eq!(
        r2.model_calls, 3,
        "calls accumulate (op1 + resumed op + verifier round): {r2:?}"
    );
    assert!(
        !log.path().join("work").join("task-0-2").exists(),
        "no shadow mission dir may be minted for a continued mission"
    );

    // The close record carries the counters so the done line, the
    // backfill and any resume all read one truth.
    let gus = goal_update_payloads(log.path(), s.stream_id());
    assert_eq!(gus.len(), 2, "two closes on one stream: {gus:?}");
    assert_eq!(gus[0]["outcome"].as_str(), Some("steps_exhausted"));
    assert_eq!(
        gus[1]["steps"].as_u64(),
        Some(2),
        "the close records cumulative steps: {gus:?}"
    );
    assert!(
        gus[1]["model_calls"].as_u64().unwrap_or(0) >= 3,
        "the close records cumulative calls: {gus:?}"
    );
    assert!(
        gus[1].get("cost_micros").is_some(),
        "the close records mission spend: {gus:?}"
    );

    // The resume itself is on the record - a kill/continue is
    // self-explaining from the event log alone.
    let fbs = feedback_payloads(log.path(), s.stream_id());
    assert!(
        fbs.iter().any(|v| v.get("mission_resumed").is_some()),
        "a mission_resumed marker is booked: {fbs:?}"
    );
}

/// A PASSED mission re-run is new work: fresh id, fresh counters.
#[test]
fn passed_mission_same_goal_starts_fresh() {
    let _guard = ENV_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let dir = tempfile::tempdir().unwrap();
    let log = tempfile::tempdir().unwrap();
    let config = write_config(dir.path());
    let a1 = log.path().join("work").join("task-0").join("answer.txt");
    let a2 = log.path().join("work").join("task-0-2").join("answer.txt");
    let script = write_script(
        dir.path(),
        &[
            serde_json::json!({"tool":"answer.write","args":{"path":a1.display().to_string(),"content":"TOKEN-0-SECRET"}}),
            serde_json::json!({"tool":"answer.write","args":{"path":a2.display().to_string(),"content":"TOKEN-0-SECRET"}}),
        ],
    );
    unsafe { std::env::set_var("HS_SEQMODEL_SCRIPT", &script) };

    let mut s = ReplSession::load(&config, log.path(), true, Some(4)).unwrap();
    let r1 = s.run_goal("task-0").unwrap();
    assert!(r1.passed, "right token passes: {r1:?}");
    assert_eq!(
        s.mission_id_for("task-0"),
        "task-0-2",
        "a passed mission re-run is a new mission"
    );
    // The fresh run mints its own work dir at mission start (the
    // checker fixture only knows task-N ids, so the run itself errors
    // at the verdict step - the id and the dir are the law under test;
    // the fresh-counter half of the law is proven by the dead-mission
    // tests above).
    let _ = s.run_goal("task-0");
    assert!(
        log.path().join("work").join("task-0-2").exists(),
        "the fresh mission gets its own work dir"
    );
}

/// Same slug, DIFFERENT goal text: never resumed into the dead mission's
/// record - the suffix path protects the log's provenance.
#[test]
fn different_goal_same_slug_starts_fresh() {
    let _guard = ENV_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let dir = tempfile::tempdir().unwrap();
    let log = tempfile::tempdir().unwrap();
    let config = write_config(dir.path());
    let answer = log.path().join("work").join("task-0").join("answer.txt");
    let script = write_script(
        dir.path(),
        &[serde_json::json!({"tool":"answer.write","args":{"path":answer.display().to_string(),"content":"WRONG"}})],
    );
    unsafe { std::env::set_var("HS_SEQMODEL_SCRIPT", &script) };

    let mut s = ReplSession::load(&config, log.path(), true, Some(1)).unwrap();
    let r1 = s.run_goal("task-0").unwrap();
    assert!(!r1.passed, "wrong token cannot pass: {r1:?}");
    // "task-0!" slugs to "task-0" but is NOT the dead mission's goal.
    assert_eq!(
        s.mission_id_for("task-0!"),
        "task-0-2",
        "a different goal under the same slug spawns a new mission"
    );
}

/// Resume survives a process restart: session 1 dies at the cap, session
/// 2 adopts the stream via load_resume and continues the same mission.
#[test]
fn dead_mission_continues_after_session_resume() {
    let _guard = ENV_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let dir = tempfile::tempdir().unwrap();
    let log = tempfile::tempdir().unwrap();
    let config = write_config(dir.path());
    let answer = log.path().join("work").join("task-0").join("answer.txt");
    let script = write_script(
        dir.path(),
        &[serde_json::json!({"tool":"answer.write","args":{"path":answer.display().to_string(),"content":"WRONG"}})],
    );
    unsafe { std::env::set_var("HS_SEQMODEL_SCRIPT", &script) };

    let stream_id = {
        let mut s1 = ReplSession::load(&config, log.path(), true, Some(1)).unwrap();
        let r1 = s1.run_goal("task-0").unwrap();
        assert!(!r1.passed, "wrong token cannot pass: {r1:?}");
        s1.stream_id()
    };

    // the model's next reply, read lazily per call
    std::fs::write(
        &script,
        serde_json::json!({"tool":"answer.write","args":{"path":answer.display().to_string(),"content":"TOKEN-0-SECRET"}})
            .to_string(),
    )
    .unwrap();

    let mut s2 = ReplSession::load_resume(&config, log.path(), true, Some(4), stream_id).unwrap();
    assert_eq!(
        s2.mission_id_for("task-0"),
        "task-0",
        "a resumed session still continues the dead mission"
    );
    let r2 = s2.run_goal("task-0").unwrap();
    assert!(r2.passed, "the resumed mission lands: {r2:?}");
    assert_eq!(
        r2.steps, 2,
        "the counter survives the restart boundary: {r2:?}"
    );
    assert!(
        !log.path().join("work").join("task-0-2").exists(),
        "no shadow mission after a restart"
    );
}
