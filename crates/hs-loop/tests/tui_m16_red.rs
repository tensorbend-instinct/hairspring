//! REPL UI gap #10 M16 (hostile self-review 2026-09-08): the resume
//! picker lists EVERY operator stream in the log root - including the
//! session you are currently inside. Offering "resume the session you
//! are already in" is nonsense at best and a state fork at worst:
//! pi/omp never list the live session in their resume pickers.
//!
//! Contract: the listing the picker is built from excludes the active
//! stream id. Excluding a foreign id changes nothing.

use hs_loop::repl::{list_sessions, list_sessions_excluding, run_one_shot};

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

// M16-r1: two missions land in one log root; the picker listing with
// the SECOND session active shows only the first. Excluding an id that
// is not in the root is a no-op.
#[test]
fn m16_picker_excludes_active_session() {
    let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
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

    let all = list_sessions(log.path());
    assert_eq!(all.len(), 2, "fixture sanity: both sessions exist: {all:?}");

    // The picker view with r2 (the live session) active.
    let picker = list_sessions_excluding(log.path(), r2.stream_id);
    assert_eq!(picker.len(), 1, "active session filtered out: {picker:?}");
    assert_eq!(picker[0].id, r1.stream_id, "only the other session remains");
    assert!(picker.iter().all(|s| s.id != r2.stream_id));

    // Symmetric: with r1 active the picker shows only r2.
    let picker = list_sessions_excluding(log.path(), r1.stream_id);
    assert_eq!(picker.len(), 1);
    assert_eq!(picker[0].id, r2.stream_id);

    // A foreign id excludes nothing.
    let picker = list_sessions_excluding(log.path(), uuid::Uuid::new_v4());
    assert_eq!(picker.len(), 2, "foreign id is a no-op: {picker:?}");
}
