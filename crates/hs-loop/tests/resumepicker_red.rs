//! REPL UI gap #7 (Eric 2026-09-08: "just fix the gaps", SOTA-REPL UI
//! investigation; look-and-feel steering 13:12): pi/omp resume prior
//! sessions from a PICKER - a list of sessions with previews. hs-repl
//! makes the operator paste a raw stream uuid into --resume.
//!
//! Contract: the REPL lists prior sessions from the log root (typed
//! `SessionInfo`: id, event count, mission preview, mtime; newest first)
//! and maps a picker's numeric selection to a stream id. `--resume`
//! with no id opens the picker on a TTY.

use hs_loop::repl::{list_sessions, pick_session, run_one_shot, session_line, SessionInfo};

const ANSWER: &str = env!("CARGO_BIN_EXE_hs-plugin-answer");
const CHECKER: &str = env!("CARGO_BIN_EXE_hs-plugin-liechecker");
const SCRIPTED: &str = env!("CARGO_BIN_EXE_hs-plugin-scripted");

static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn scripted_config(dir: &std::path::Path) -> std::path::PathBuf {
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

fn fake_info(id: &str, events: u64, preview: &str) -> SessionInfo {
    SessionInfo {
        id: uuid::Uuid::parse_str(id).unwrap(),
        events,
        preview: preview.to_string(),
        modified: std::time::SystemTime::UNIX_EPOCH,
    }
}

// R1: two one-shot missions in one log root list as two sessions,
// NEWEST FIRST, each with its goal text as the preview.
#[test]
fn r1_lists_sessions_newest_first() {
    let _guard = ENV_LOCK.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let dir = tempfile::tempdir().unwrap();
    let log = tempfile::tempdir().unwrap();
    let config = scripted_config(dir.path());
    let script = dir.path().join("s.jsonl");
    let a1 = log.path().join("work").join(hs_loop::repl::goal_slug("alpha goal")).join("answer.txt");
    let a2 = log.path().join("work").join(hs_loop::repl::goal_slug("beta goal")).join("answer.txt");
    std::fs::write(
        &script,
        format!(
            "{{\"tool\":\"answer.write\",\"args\":{{\"path\":\"{}\",\"content\":\"A\"}}}}\n{{\"tool\":\"answer.write\",\"args\":{{\"path\":\"{}\",\"content\":\"B\"}}}}",
            a1.display(),
            a2.display()
        ),
    )
    .unwrap();
    unsafe { std::env::set_var("HS_SEQMODEL_SCRIPT", &script) };
    let r1 = run_one_shot(&config, log.path(), "alpha goal", false, 2).unwrap();
    std::thread::sleep(std::time::Duration::from_millis(1100));
    let r2 = run_one_shot(&config, log.path(), "beta goal", false, 2).unwrap();

    let sessions = list_sessions(log.path());
    assert_eq!(sessions.len(), 2, "both streams listed: {sessions:?}");
    assert_eq!(sessions[0].id, r2.stream_id, "newest first");
    assert_eq!(sessions[1].id, r1.stream_id);
    assert!(sessions[0].preview.contains("beta goal"), "preview carries the goal: {:?}", sessions[0].preview);
    assert!(sessions[1].preview.contains("alpha goal"));
    assert!(sessions[0].events >= 1, "event count recorded");
}

// R2: numeric picker selection maps to stream ids; junk selects nothing.
#[test]
fn r2_pick_session() {
    let infos = vec![
        fake_info("4c4056c0-ced7-4d5d-9e6a-a85d29a593b9", 5, "beta goal"),
        fake_info("20c0dff2-3b58-4b95-bad8-e9f2d3812c82", 3, "alpha goal"),
    ];
    assert_eq!(
        pick_session(&infos, "1"),
        Some(uuid::Uuid::parse_str("4c4056c0-ced7-4d5d-9e6a-a85d29a593b9").unwrap())
    );
    assert_eq!(
        pick_session(&infos, "2"),
        Some(uuid::Uuid::parse_str("20c0dff2-3b58-4b95-bad8-e9f2d3812c82").unwrap())
    );
    assert_eq!(pick_session(&infos, "0"), None);
    assert_eq!(pick_session(&infos, "3"), None);
    assert_eq!(pick_session(&infos, "abc"), None);
    assert_eq!(pick_session(&infos, ""), None);
}

// R3: the display line carries the short id, event count, and preview.
#[test]
fn r3_session_line() {
    let info = fake_info("4c4056c0-ced7-4d5d-9e6a-a85d29a593b9", 7, "fix the parser");
    let line = session_line(1, &info);
    assert!(line.starts_with("1)"), "numbered: {line:?}");
    assert!(line.contains("4c4056c0"), "short id: {line:?}");
    assert!(line.contains("7 events"), "event count: {line:?}");
    assert!(line.contains("fix the parser"), "preview: {line:?}");
}

// R4 (live defect, found by the picker proof): the interactive REPL
// ignored --resume entirely and opened a FRESH stream. The shared
// constructor must adopt the picked stream in every mode.
#[test]
fn r4_load_session_resume_adopts_stream() {
    let _guard = ENV_LOCK.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let dir = tempfile::tempdir().unwrap();
    let log = tempfile::tempdir().unwrap();
    let config = scripted_config(dir.path());
    let script = dir.path().join("s2.jsonl");
    let a1 = log.path().join("work").join(hs_loop::repl::goal_slug("first run")).join("answer.txt");
    std::fs::write(
        &script,
        format!("{{\"tool\":\"answer.write\",\"args\":{{\"path\":\"{}\",\"content\":\"X\"}}}}", a1.display()),
    )
    .unwrap();
    unsafe { std::env::set_var("HS_SEQMODEL_SCRIPT", &script) };
    let first = hs_loop::repl::load_session(&config, log.path(), false, 2, None, None)
        .expect("fresh session");
    let r = { let mut s = first; s.run_goal("first run").unwrap() };

    let resumed = hs_loop::repl::load_session(&config, log.path(), false, 2, Some(r.stream_id), None)
        .expect("resumed session");
    assert_eq!(
        resumed.stream_id(),
        r.stream_id,
        "resume adopts the picked stream instead of opening a fresh one"
    );

    // resume and fork are exclusive
    assert!(hs_loop::repl::load_session(&config, log.path(), false, 2, Some(r.stream_id), Some(r.stream_id)).is_err());
}
