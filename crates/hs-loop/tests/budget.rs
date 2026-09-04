//! Gate-8 budget enforcement on the inner loop (red): a mission whose
//! model calls would exceed its USD budget is killed at the cap and the
//! result is flagged, so budget-killed missions score as failures.

use hs_loop::*;

const ANSWER: &str = env!("CARGO_BIN_EXE_hs-plugin-answer");
const CHECKER: &str = env!("CARGO_BIN_EXE_hs-plugin-checker");
const BENCHMODEL: &str = env!("CARGO_BIN_EXE_hs-plugin-benchmodel");

#[test]
fn mission_is_killed_at_budget_cap() {
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
name = "checker.run"
command = ["{CHECKER}"]
subjects = ["*"]

[[models]]
name = "benchmodel"
command = ["{BENCHMODEL}"]
default = true
"#
        ),
    )
    .unwrap();
    let kernel = hs_kernel::Kernel::load(&config).unwrap();
    let mut l = InnerLoop::new(kernel, log.path(), true, 50).unwrap();
    // benchmodel reports 900 micro-USD per call; a 5000-micro cap must kill
    // the mission after 5 calls, long before the 50-step cap
    l.set_budget_micros(5_000);
    let r = l.run_mission("task-0").unwrap();
    assert!(!r.passed, "budget-killed mission is a failure");
    assert!(r.budget_killed, "result must flag the budget kill");
    assert!(r.model_calls <= 6, "killed near the cap, got {}", r.model_calls);
    assert!(l.total_cost_micros() <= 5_900);
}

#[test]
fn mission_under_budget_runs_normally() {
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
name = "checker.run"
command = ["{CHECKER}"]
subjects = ["*"]

[[models]]
name = "benchmodel"
command = ["{BENCHMODEL}"]
default = true
"#
        ),
    )
    .unwrap();
    let kernel = hs_kernel::Kernel::load(&config).unwrap();
    let mut l = InnerLoop::new(kernel, log.path(), true, 6).unwrap();
    l.set_budget_micros(10_000_000); // $10: never fires
    let r = l.run_mission("task-0").unwrap();
    assert!(r.passed);
    assert!(!r.budget_killed);
}
