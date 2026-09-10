//! BURN-DOWN RED (Eric 2026-09-09: the figure's spine must be true from
//! the surface he uses): the INDEPENDENT critic runs on the default TUI
//! mission path, with its instruction anchor, and the mission records its
//! completion mode (spec 5.2: self|independent|hybrid - the TUI default
//! is hybrid: say-so triggers the checkers, the checkers decide).

use hs_loop::repl::ReplSession;

const ANSWER: &str = env!("CARGO_BIN_EXE_hs-plugin-answer");
const CRITIC: &str = env!("CARGO_BIN_EXE_hs-plugin-critic");
const TERMEXEC: &str = env!("CARGO_BIN_EXE_hs-plugin-termexec");
const SCRIPTED: &str = env!("CARGO_BIN_EXE_hs-plugin-scripted");

static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn stream_ledger(log_root: &std::path::Path, sid: uuid::Uuid) -> String {
    let reader = hs_log::StreamReader::open(log_root, sid).unwrap();
    let mut all = String::new();
    for ev in reader.events().unwrap() {
        all.push_str(&format!("{:?} ", ev.kind));
        if let Ok(b) = reader.resolve_payload(&ev) {
            all.push_str(&String::from_utf8_lossy(&b));
        }
        all.push('\n');
    }
    all
}

#[test]
fn tui_default_path_runs_independent_critic_and_records_mode() {
    let _guard = ENV_LOCK.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let dir = tempfile::tempdir().unwrap();
    let log = tempfile::tempdir().unwrap();
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
name = "term.exec"
command = ["{TERMEXEC}"]
subjects = ["*"]

[[tools]]
name = "checker.run"
command = ["{CRITIC}"]
subjects = ["*"]

[[models]]
name = "scripted"
command = ["{SCRIPTED}"]
default = true
"#
        ),
    )
    .unwrap();

    // The live-surface declared checks (critic gate phase 1): green.
    let work = log.path().join("work");
    std::fs::create_dir_all(work.join(".hs")).unwrap();
    std::fs::write(work.join(".hs/checks"), "true\n").unwrap();

    let goal = "write the token file";
    let answer = work.join(hs_loop::repl::goal_slug(goal)).join("answer.txt");
    let script = dir.path().join("model.jsonl");
    std::fs::write(
        &script,
        format!(
            // line 1: sacrificial (kernel probes the model plugin at load)
            "{{\"tool\":\"answer.submit\",\"args\":{{\"path\":\"probe\",\"content\":\"probe\"}}}}\n\
             {{\"tool\":\"answer.write\",\"args\":{{\"path\":\"{}\",\"content\":\"TOKEN\"}}}}\n\
             {{\"tool\":\"verdict.submit\",\"args\":{{\"refuted\":false,\"blocking\":\"none\",\"findings\":[]}}}}",
            answer.display()
        ),
    )
    .unwrap();
    let trace = dir.path().join("critic-trace.jsonl");
    unsafe {
        std::env::set_var("HS_SEQMODEL_SCRIPT", &script);
        std::env::set_var("HS_CRITIC_SCRIPT", "tool:cat .hs/instruction.txt|clean");
        std::env::set_var("HS_CRITIC_TRACE", &trace);
        std::env::remove_var("HS_TB_INSTRUCTION_FILE");
    }

    let mut session = ReplSession::load(&config, log.path(), false, 6).expect("session load");
    let r = session.run_goal(goal).expect("mission runs");
    assert!(r.passed, "critic-gated TUI mission passes: {r:?}");

    // 1. the critic's instruction anchor exists, written by the session
    let inst = std::fs::read_to_string(work.join(".hs/instruction.txt")).expect(
        "the TUI must write the mission instruction anchor for the critic gate (no HS_TB_INSTRUCTION_FILE on this path)",
    );
    assert!(inst.contains(goal), "the anchor carries the goal text: {inst}");

    // 2. the critic REALLY ran - not just the selfcheck phase
    let trace_text = std::fs::read_to_string(&trace)
        .expect("critic trace exists: the critic phase ran on the TUI path");
    assert!(
        trace_text.contains("\"verdict\""),
        "a verdict landed in the critic trace: {trace_text}"
    );

    // 3. completion mode recorded on the mission close (spec 5.2)
    let ledger = stream_ledger(log.path(), r.stream_id);
    assert!(
        ledger.contains("\"completion_mode\":\"hybrid\""),
        "the close books the spec's completion mode; ledger: {ledger}"
    );

    unsafe {
        std::env::remove_var("HS_CRITIC_SCRIPT");
        std::env::remove_var("HS_CRITIC_TRACE");
    }
}

#[test]
fn close_books_intervention_detection_repair_accounting() {
    let _guard = ENV_LOCK.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let dir = tempfile::tempdir().unwrap();
    let log = tempfile::tempdir().unwrap();
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
name = "term.exec"
command = ["{TERMEXEC}"]
subjects = ["*"]

[[tools]]
name = "checker.run"
command = ["{CRITIC}"]
subjects = ["*"]

[[models]]
name = "scripted"
command = ["{SCRIPTED}"]
default = true
"#
        ),
    )
    .unwrap();

    // The declared checks START RED: the first verdict is an intervention
    // with reproduced evidence; the model repairs them and resubmits.
    let work = log.path().join("work");
    std::fs::create_dir_all(work.join(".hs")).unwrap();
    std::fs::write(work.join(".hs/checks"), "false\n").unwrap();

    let goal = "write the token file";
    let answer = work.join(hs_loop::repl::goal_slug(goal)).join("answer.txt");
    let script = dir.path().join("model.jsonl");
    std::fs::write(
        &script,
        format!(
            "{{\"tool\":\"answer.submit\",\"args\":{{\"path\":\"probe\",\"content\":\"probe\"}}}}\n\
             {{\"tool\":\"answer.write\",\"args\":{{\"path\":\"{0}\",\"content\":\"TOKEN\"}}}}\n\
             {{\"tool\":\"term.exec\",\"args\":{{\"command\":\"printf 'true\\\\n' > .hs/checks\"}}}}\n\
             {{\"tool\":\"answer.write\",\"args\":{{\"path\":\"{0}\",\"content\":\"TOKEN\"}}}}\n\
             {{\"tool\":\"verdict.submit\",\"args\":{{\"refuted\":false,\"blocking\":\"none\",\"findings\":[]}}}}",
            answer.display()
        ),
    )
    .unwrap();
    unsafe {
        std::env::set_var("HS_SEQMODEL_SCRIPT", &script);
        std::env::set_var("HS_CRITIC_SCRIPT", "tool:cat .hs/instruction.txt|clean");
    }

    let mut session = ReplSession::load(&config, log.path(), false, 8).expect("session load");
    let r = session.run_goal(goal).expect("mission runs");
    assert!(r.passed, "repaired mission closes verified: {r:?}");

    let ledger = stream_ledger(log.path(), r.stream_id);
    for needle in [
        "\"interventions\":1",
        "\"detections\":1",
        "\"repairs\":1",
        "\"repair_rate\":1.0",
    ] {
        assert!(ledger.contains(needle), "close books {needle}; ledger: {ledger}");
    }

    unsafe {
        std::env::remove_var("HS_CRITIC_SCRIPT");
    }
}
