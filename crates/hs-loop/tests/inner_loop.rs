//! Gate-3 TDD: inner loop contract (spec section 6), written first.

use hs_loop::*;

const ANSWER: &str = env!("CARGO_BIN_EXE_hs-plugin-answer");
const CHECKER: &str = env!("CARGO_BIN_EXE_hs-plugin-checker");
const BENCHMODEL: &str = env!("CARGO_BIN_EXE_hs-plugin-benchmodel");

fn rig(dir: &std::path::Path, log: &std::path::Path, feedback: bool) -> InnerLoop {
    let config = dir.join("hairspring.toml");
    std::fs::write(&config, format!(r#"
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
"#)).unwrap();
    let kernel = hs_kernel::Kernel::load(&config).unwrap();
    InnerLoop::new(kernel, log, feedback, 6).unwrap()
}

#[test]
fn feedback_on_repairs_repairable_task_in_two_steps() {
    let dir = tempfile::tempdir().unwrap();
    let log = tempfile::tempdir().unwrap();
    let mut l = rig(dir.path(), log.path(), true);
    let r = l.run_mission("task-0").unwrap();
    assert!(r.passed, "repairable task must pass with feedback on");
    assert_eq!(r.steps, 2, "blind attempt, then feedback-guided repair");
    assert_eq!(r.model_calls, r.steps, "feedback adds zero extra model round trips");
}

#[test]
fn feedback_off_never_repairs() {
    let dir = tempfile::tempdir().unwrap();
    let log = tempfile::tempdir().unwrap();
    let mut l = rig(dir.path(), log.path(), false);
    let r = l.run_mission("task-0").unwrap();
    assert!(!r.passed);
    assert_eq!(r.steps, 6, "ran to the step cap");
}

#[test]
fn unrepairable_failure_class_fails_even_with_feedback_on() {
    let dir = tempfile::tempdir().unwrap();
    let log = tempfile::tempdir().unwrap();
    let mut l = rig(dir.path(), log.path(), true);
    // task-18..task-23 are the unrepairable family (checker error carries no fix)
    let r = l.run_mission("task-18").unwrap();
    assert!(!r.passed, "feedback cannot hide this failure class");
}

#[test]
fn events_tell_the_two_arms_apart() {
    let dir = tempfile::tempdir().unwrap();
    let log_on = tempfile::tempdir().unwrap();
    let log_off = tempfile::tempdir().unwrap();
    rig(dir.path(), log_on.path(), true).run_mission("task-0").unwrap();
    rig(dir.path(), log_off.path(), false).run_mission("task-0").unwrap();
    let on = events(log_on.path());
    let off = events(log_off.path());
    for (name, evs) in [("on", &on), ("off", &off)] {
        assert!(evs.iter().any(|e| e.kind == hs_core::EventKind::Feedback),
            "{name}: checker verdicts must be recorded in BOTH arms");
    }
    assert!(on.iter().any(|e| e.kind == hs_core::EventKind::ContextInject),
        "ON arm records what entered the window and why");
    assert!(!off.iter().any(|e| e.kind == hs_core::EventKind::ContextInject),
        "OFF arm injects nothing");
    // both chains verify end to end
    for lg in [log_on.path(), log_off.path()] {
        let sid = only_stream(lg);
        hs_log::verify_stream(lg, sid).unwrap();
    }
}

fn events(log: &std::path::Path) -> Vec<hs_core::Event> {
    let sid = only_stream(log);
    hs_log::StreamReader::open(log, sid).unwrap().events().unwrap()
}
fn only_stream(log: &std::path::Path) -> uuid::Uuid {
    let mut v: Vec<_> = std::fs::read_dir(log.join("streams")).unwrap()
        .map(|e| e.unwrap().file_name().into_string().unwrap()).collect();
    assert_eq!(v.len(), 1);
    uuid::Uuid::parse_str(&v.pop().unwrap()).unwrap()
}
