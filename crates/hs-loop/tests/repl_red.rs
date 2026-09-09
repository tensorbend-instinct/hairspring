//! RED acceptance gates for the REPL (Eric 2026-09-07: "build the repl for
//! hairspring so anyone can run the harness via cli on a goal end to end").
//!
//! R1 `parse_command`: REPL input lines classify into goal text vs :commands.
//! R2 `one_shot_goal_end_to_end`: one API call loads a kernel, runs a mission
//!    on a goal, and returns the `MissionResult` with the answer on disk.
//! R3 `session_reuse`: a session runs two goals on one kernel, distinct mission
//!    ids, and `last_answer()` reads the latest answer.

use hs_loop::repl::{parse_command, run_one_shot, ReplCommand, ReplSession};

const ANSWER: &str = env!("CARGO_BIN_EXE_hs-plugin-answer");
const CHECKER: &str = env!("CARGO_BIN_EXE_hs-plugin-liechecker");
const SCRIPTED: &str = env!("CARGO_BIN_EXE_hs-plugin-scripted");

// HS_SEQMODEL_SCRIPT is process-global: serialize the mission tests.
static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

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

// R1: command parsing
#[test]
fn r1_parse_command() {
    assert_eq!(parse_command(":quit"), ReplCommand::Quit);
    assert_eq!(parse_command(":q"), ReplCommand::Quit);
    assert_eq!(parse_command(":help"), ReplCommand::Help);
    assert_eq!(parse_command(":status"), ReplCommand::Status);
    assert_eq!(parse_command(":last"), ReplCommand::LastAnswer);
    assert_eq!(
        parse_command("fix the off-by-one in parse_patch"),
        ReplCommand::Goal("fix the off-by-one in parse_patch".into())
    );
    assert_eq!(
        parse_command("  fix spacing  "),
        ReplCommand::Goal("fix spacing".into())
    );
    // unknown colon-command is a goal-shaped error, not a panic
    match parse_command(":bogus") {
        ReplCommand::Unknown(c) => assert_eq!(c, ":bogus"),
        other => panic!("expected Unknown, got {other:?}"),
    }
}

// R1b: mission ids from goal text are path-safe and distinct
#[test]
fn r1b_goal_slug() {
    let a = hs_loop::repl::goal_slug("Fix the parser's off-by-one!");
    assert!(
        a.chars().all(|c| c.is_ascii_alphanumeric() || c == '-'),
        "path-safe slug: {a}"
    );
    assert!(!a.is_empty());
    assert_ne!(
        hs_loop::repl::goal_slug("first goal"),
        hs_loop::repl::goal_slug("second goal")
    );
}

// R2: one shot, end to end
#[test]
fn r2_one_shot_goal_end_to_end() {
    let _guard = ENV_LOCK.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let dir = tempfile::tempdir().unwrap();
    let log = tempfile::tempdir().unwrap();
    let config = write_config(dir.path());
    let script = dir.path().join("script.jsonl");
    std::fs::write(
        &script,
        r#"{"tool":"answer.write","args":{"path":"ANSWER_PATH","content":"REPL-TOKEN-42"}}"#
            .replace(
                "ANSWER_PATH",
                &log.path()
                    .join("work")
                    .join(hs_loop::repl::goal_slug("write the token"))
                    .join("answer.txt")
                    .display()
                    .to_string(),
            ),
    )
    .unwrap();
    unsafe { std::env::set_var("HS_SEQMODEL_SCRIPT", &script) };

    let r = run_one_shot(&config, log.path(), "write the token", false, 3).expect("one-shot run");
    assert!(r.steps >= 1, "at least one step ran: {r:?}");
    let answer = std::fs::read_to_string(&r.answer_path).unwrap();
    assert!(
        answer.contains("REPL-TOKEN-42"),
        "the goal's answer landed on disk at {}: {answer}",
        r.answer_path.display()
    );
}

// R3: a session runs two goals on one loaded kernel
#[test]
fn r3_session_reuse() {
    let _guard = ENV_LOCK.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let dir = tempfile::tempdir().unwrap();
    let log = tempfile::tempdir().unwrap();
    let config = write_config(dir.path());

    // the scripted model reads HS_SEQMODEL_SCRIPT once at spawn and
    // consumes it across the whole session: one script, one line per goal
    let a1 = log
        .path()
        .join("work")
        .join(hs_loop::repl::goal_slug("goal one"))
        .join("answer.txt");
    let a2 = log
        .path()
        .join("work")
        .join(hs_loop::repl::goal_slug("goal two"))
        .join("answer.txt");
    let script = dir.path().join("session.jsonl");
    std::fs::write(
        &script,
        format!(
            "{{\"tool\":\"answer.write\",\"args\":{{\"path\":\"{}\",\"content\":\"TOKEN-ONE\"}}}}\n{{\"tool\":\"answer.write\",\"args\":{{\"path\":\"{}\",\"content\":\"TOKEN-TWO\"}}}}",
            a1.display(),
            a2.display()
        ),
    )
    .unwrap();
    unsafe { std::env::set_var("HS_SEQMODEL_SCRIPT", &script) };
    let mut session = ReplSession::load(&config, log.path(), false, 3).expect("session load");
    assert_eq!(
        session.mission_id_for("goal one"),
        hs_loop::repl::goal_slug("goal one"),
        "first run of a goal uses the plain slug"
    );
    let r1 = session.run_goal("goal one").expect("first goal");
    assert_eq!(
        std::fs::read_to_string(&r1.answer_path).unwrap(),
        "TOKEN-ONE"
    );

    let r2 = session.run_goal("goal two").expect("second goal");
    assert_ne!(r1.answer_path, r2.answer_path, "distinct mission dirs");
    assert_eq!(
        session.last_answer().as_deref(),
        Some("TOKEN-TWO"),
        "last_answer reads the latest mission"
    );
}
