//! Gate-4 TDD: outer loop contract, written first.

use hs_goal::*;

const ANSWER: &str = env!("CARGO_BIN_EXE_hs-plugin-goalanswer");
const GOALCHECKER: &str = env!("CARGO_BIN_EXE_hs-plugin-goalchecker");
const GOALMODEL: &str = env!("CARGO_BIN_EXE_hs-plugin-goalmodel");

fn rig(dir: &std::path::Path, log: &std::path::Path) -> OuterLoop {
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
name = "goalchecker.run"
command = ["{GOALCHECKER}"]
subjects = ["*"]

[[models]]
name = "goalmodel"
command = ["{GOALMODEL}"]
default = true
"#
        ),
    )
    .unwrap();
    let kernel = hs_kernel::Kernel::load(&config).unwrap();
    OuterLoop::new(kernel, log, 0).unwrap()
}

fn goal(spec: &str) -> Goal {
    Goal::new(
        spec,
        CompletionMode::SelfDeclared,
        Budget {
            max_steps: 8,
            max_cost_usd_micros: 1_000_000,
        },
    )
}

/// Offline ground truth: what the hidden test requires. The benchmark
/// harness (not the loop under test) uses it to count self-mode misses.
fn hidden_correct(spec: &str) -> String {
    let i: usize = spec.strip_prefix("plant-").unwrap().parse().unwrap();
    format!("VISIBLE-{i}\nHIDDEN-{i}")
}

fn artifact_of(log: &std::path::Path, spec: &str) -> String {
    std::fs::read_to_string(log.join("work").join(spec).join("answer.txt")).unwrap_or_default()
}

// Planted false completion: plant-1 (odd) - artifact meets the visible
// spec, fails the hidden test, model declares done at attempt 2.
#[test]
fn self_mode_accepts_a_false_completion_and_it_is_counted() {
    let dir = tempfile::tempdir().unwrap();
    let log = tempfile::tempdir().unwrap();
    let mut l = rig(dir.path(), log.path());
    let out = l.run(&goal("plant-1")).unwrap();
    assert!(
        matches!(out, MissionOutcome::Passed { .. }),
        "self mode trusts the say-so"
    );
    // the miss is measured against offline ground truth, not by the loop
    assert_ne!(
        artifact_of(log.path(), "plant-1").trim(),
        hidden_correct("plant-1").trim(),
        "expected a false artifact in this plant"
    );
    assert_eq!(l.completions_accepted_on_say_so(), 1);
}

#[test]
fn independent_mode_catches_every_false_completion() {
    let dir = tempfile::tempdir().unwrap();
    let log = tempfile::tempdir().unwrap();
    let mut l = rig(dir.path(), log.path());
    let mut g = goal("plant-1");
    g.completion_mode = CompletionMode::Independent;
    let out = l.run(&g).unwrap();
    assert!(
        !matches!(out, MissionOutcome::Passed { .. }),
        "independent mode reported a false pass: gate falsified"
    );
    // independent mode has no say-so to catch; never reporting the false
    // pass IS the catch. The catch counter belongs to hybrid mode.
}

#[test]
fn hybrid_mode_catches_false_completion_but_passes_honest_work() {
    let dir = tempfile::tempdir().unwrap();
    let log1 = tempfile::tempdir().unwrap();
    let mut l = rig(dir.path(), log1.path());
    let mut g = goal("plant-1");
    g.completion_mode = CompletionMode::Hybrid;
    let out = l.run(&g).unwrap();
    assert!(
        !matches!(out, MissionOutcome::Passed { .. }),
        "hybrid reported a false pass"
    );
    assert!(
        l.false_completions_caught() >= 1,
        "the catch must be counted"
    );

    // plant-0 is honest: the artifact passes hidden tests when done is declared
    let log0 = tempfile::tempdir().unwrap();
    let mut l0 = rig(dir.path(), log0.path());
    let mut g0 = goal("plant-0");
    g0.completion_mode = CompletionMode::Hybrid;
    let out0 = l0.run(&g0).unwrap();
    assert!(
        matches!(out0, MissionOutcome::Passed { .. }),
        "hybrid must not block honest completion"
    );
}

#[test]
fn independent_mode_still_passes_honest_work() {
    let dir = tempfile::tempdir().unwrap();
    let log = tempfile::tempdir().unwrap();
    let mut l = rig(dir.path(), log.path());
    let mut g = goal("plant-0");
    g.completion_mode = CompletionMode::Independent;
    let out = l.run(&g).unwrap();
    assert!(matches!(out, MissionOutcome::Passed { .. }));
}

#[test]
fn budget_exceeded_checkpoints_and_stops() {
    let dir = tempfile::tempdir().unwrap();
    let log = tempfile::tempdir().unwrap();
    let mut l = rig(dir.path(), log.path());
    let mut g = goal("plant-1");
    g.budget.max_cost_usd_micros = 50; // one model call costs 900 micros
    let out = l.run(&g).unwrap();
    assert!(
        matches!(out, MissionOutcome::BudgetExceeded { .. }),
        "{out:?}"
    );
    let sid = l.stream_id();
    let events = hs_log::StreamReader::open(log.path(), sid)
        .unwrap()
        .events()
        .unwrap();
    assert!(
        events
            .iter()
            .any(|e| e.kind == hs_core::EventKind::BudgetUpdate),
        "budget event missing"
    );
    assert!(
        events
            .iter()
            .any(|e| e.kind == hs_core::EventKind::Decision),
        "checkpoint breakpoint missing"
    );
    hs_log::verify_stream(log.path(), sid).unwrap();
}

#[test]
fn gateway_cancel_mid_run_leaves_progress_intact() {
    let dir = tempfile::tempdir().unwrap();
    let log = tempfile::tempdir().unwrap();
    let mut l = rig(dir.path(), log.path()).with_step_delay_ms(80);
    let inbox = l.gateway_inbox();
    let mut g = goal("plant-1");
    g.completion_mode = CompletionMode::Hybrid;
    let t = std::thread::spawn(move || l.run(&g));
    std::thread::sleep(std::time::Duration::from_millis(200));
    write_gateway(&inbox, &serde_json::json!({"type": "cancel"}));
    let out = t.join().unwrap().unwrap();
    let MissionOutcome::Cancelled { at_step, .. } = out else {
        panic!("expected cancel, got {out:?}")
    };
    assert!(at_step < 8);
    // progress intact: artifact exists, every event so far is on a verified chain
    let lg = tempfile::tempdir().unwrap(); // log dir was moved into the loop; recover via inbox parent
    let _ = lg;
    let log_root = inbox.parent().unwrap().to_path_buf();
    let mut streams: Vec<_> = std::fs::read_dir(log_root.join("streams"))
        .unwrap()
        .map(|e| e.unwrap().file_name().into_string().unwrap())
        .collect();
    let sid = uuid::Uuid::parse_str(&streams.pop().unwrap()).unwrap();
    hs_log::verify_stream(&log_root, sid).unwrap();
    let events = hs_log::StreamReader::open(&log_root, sid)
        .unwrap()
        .events()
        .unwrap();
    assert!(
        events.iter().any(|e| e.kind == hs_core::EventKind::Message),
        "cancel not recorded"
    );
    assert!(
        events
            .iter()
            .any(|e| e.kind == hs_core::EventKind::ToolCall),
        "pre-cancel work is not on the log"
    );
}

#[test]
fn gateway_redirect_mid_run_retargets_without_losing_history() {
    let dir = tempfile::tempdir().unwrap();
    let log = tempfile::tempdir().unwrap();
    let mut l = rig(dir.path(), log.path()).with_step_delay_ms(80);
    let inbox = l.gateway_inbox();
    let mut g = goal("plant-1");
    g.completion_mode = CompletionMode::Hybrid;
    let t = std::thread::spawn(move || l.run(&g));
    std::thread::sleep(std::time::Duration::from_millis(200));
    write_gateway(
        &inbox,
        &serde_json::json!({"type": "redirect", "new_spec": "plant-0"}),
    );
    let out = t.join().unwrap().unwrap();
    assert!(
        matches!(out, MissionOutcome::Passed { .. }),
        "redirect to an honest plant should pass, got {out:?}"
    );
    let log_root = inbox.parent().unwrap().to_path_buf();
    let mut streams: Vec<_> = std::fs::read_dir(log_root.join("streams"))
        .unwrap()
        .map(|e| e.unwrap().file_name().into_string().unwrap())
        .collect();
    let sid = uuid::Uuid::parse_str(&streams.pop().unwrap()).unwrap();
    hs_log::verify_stream(&log_root, sid).unwrap();
    let events = hs_log::StreamReader::open(&log_root, sid)
        .unwrap()
        .events()
        .unwrap();
    assert!(
        events
            .iter()
            .any(|e| e.kind == hs_core::EventKind::GoalUpdate),
        "redirect not recorded as goal_update"
    );
    // work before AND after the redirect shares one unbroken chain
    let seqs: Vec<u64> = events.iter().map(|e| e.seq).collect();
    assert!(seqs.windows(2).all(|w| w[1] == w[0] + 1));
}

fn write_gateway(inbox: &std::path::Path, v: &serde_json::Value) {
    use std::io::Write;
    let mut f = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(inbox)
        .unwrap();
    writeln!(f, "{}", v).unwrap();
    f.sync_all().unwrap();
}
