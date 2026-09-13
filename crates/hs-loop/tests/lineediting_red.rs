//! REPL PARITY GAP #5: line editing - input history that survives restart.
//!
//! pi/omp-class REPLs give you a real input line: cursor movement, and
//! up-arrow history that is still there when you come back tomorrow.
//! hs-repl's interactive mode reads bare stdin lines - no editing, no
//! history, nothing persisted. That is the red.
//!
//! Contract (the ownable, testable slice):
//! - every accepted line (goals AND commands) lands in a persistent
//!   history file inside the session dir, in order;
//! - a NEW interactive session on the same dir preloads that history
//!   (up-arrow across restarts);
//! - piped stdin keeps working (non-TTY fallback), and ALSO records
//!   history.
//!
//! Editing proper (cursor movement, kill ring) is rustyline's domain
//! on a real TTY; what HAIRSPRING must own is the history lifecycle.

use hs_loop::repl::{Editor, ReplSession, StdinEditor};

static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

const ANSWER: &str = env!("CARGO_BIN_EXE_hs-plugin-answer");
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

fn script(dir: &std::path::Path, lines: &[serde_json::Value]) {
    std::fs::write(
        dir.join("s.jsonl"),
        lines.iter().map(std::string::ToString::to_string).collect::<Vec<_>>().join("\n"),
    )
    .unwrap();
    unsafe { std::env::set_var("HS_SEQMODEL_SCRIPT", dir.join("s.jsonl")) };
}

#[test]
fn interactive_history_persists_and_preloads_across_restarts() {
    let _guard = ENV_LOCK.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let dir = tempfile::tempdir().unwrap();
    let log = tempfile::tempdir().unwrap();
    let config = write_config(dir.path());

    // Session 1: run one goal, then quit - via the piped (non-TTY) editor.
    script(
        dir.path(),
        &[serde_json::json!({"tool":"answer.write","args":{"path":log.path().join("work/t1/answer.txt").display().to_string(),"content":"x"}})],
    );
    {
        let mut session = ReplSession::load(&config, log.path(), true, Some(1)).unwrap();
        let mut editor = StdinEditor::new(log.path(), "first goal line\n:quit\n".as_bytes());
        hs_loop::repl::run_interactive(&mut session, &mut editor).unwrap();
    }

    // History file exists in the session dir, in order.
    let hist_path = log.path().join(".hs_repl_history");
    let body = std::fs::read_to_string(&hist_path)
        .expect("the interactive session records a persistent history file");
    let lines: Vec<&str> = body.lines().collect();
    assert_eq!(
        lines,
        vec!["first goal line", ":quit"],
        "history records accepted lines in order: {lines:?}"
    );

    // Session 2 (the restart): the editor preloads session-1 history.
    let editor2 = StdinEditor::new(log.path(), ":quit\n".as_bytes());
    assert_eq!(
        editor2.history(),
        &["first goal line".to_string(), ":quit".to_string()],
        "a restarted session preloads prior history (up-arrow works across restarts)"
    );
}
