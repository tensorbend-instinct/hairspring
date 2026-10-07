//! Wall-clock guard enforcement on the inner loop (red). Live burn
//! 2026-09-07: the glm-critic TB trial ran past its 2h wall because
//! --wall-secs only fed the T-minus display; enforcement had been
//! delegated to the harness's external exec timeout (wall + 600s), which
//! killed the container and destroyed every artifact (ledger, critic
//! trace, checks, answer). The wall guard must fire INSIDE the loop: the
//! mission stops at its wall, books outcome "`wall_killed`" with its steps
//! and cost preserved, and exits cleanly so artifacts survive and the
//! official verifier still grades the final machine state.

use hs_loop::*;

const ANSWER: &str = env!("CARGO_BIN_EXE_hs-plugin-answer");
const CHECKER: &str = env!("CARGO_BIN_EXE_hs-plugin-checker");
const BENCHMODEL: &str = env!("CARGO_BIN_EXE_hs-plugin-benchmodel");

fn wall_kernel(dir: &tempfile::TempDir) -> hs_kernel::Kernel {
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
    hs_kernel::Kernel::load(&config).unwrap()
}

#[test]
fn mission_is_killed_at_wall_guard() {
    // feedback OFF: benchmodel never repairs, so without a wall guard this
    // mission runs to the 50-step cap. wall_secs = 0 must kill it at the
    // first step boundary instead - booked wall_killed, steps preserved,
    // never a 0-step row for a mission that burned a real model call.
    let dir = tempfile::tempdir().unwrap();
    let log = tempfile::tempdir().unwrap();
    let kernel = wall_kernel(&dir);
    let mut l = InnerLoop::new(kernel, log.path(), false, 50).unwrap();
    l.set_budget_micros(10_000_000);
    l.set_wall_secs(0);
    let r = l.run_mission("task-0").unwrap();
    assert!(!r.passed, "wall-killed mission is a failure");
    assert!(!r.budget_killed, "wall kill is not a budget kill");
    assert_eq!(
        r.outcome, "wall_killed",
        "wall guard books wall_killed, got {}",
        r.outcome
    );
    assert!(
        r.steps <= 1,
        "killed at the first step boundary, got {} steps",
        r.steps
    );
    assert_eq!(r.model_calls, r.steps, "accounting stays consistent");
}

#[test]
fn generous_wall_does_not_interfere() {
    // Same fixture with a wall the mission can never reach: the mission
    // must die by its step cap exactly as before - the guard changes
    // nothing for missions inside their wall.
    let dir = tempfile::tempdir().unwrap();
    let log = tempfile::tempdir().unwrap();
    let kernel = wall_kernel(&dir);
    let mut l = InnerLoop::new(kernel, log.path(), false, 50).unwrap();
    l.set_budget_micros(10_000_000);
    l.set_wall_secs(3600);
    let r = l.run_mission("task-0").unwrap();
    assert_eq!(
        r.outcome, "steps_exhausted",
        "generous wall must not fire, got {}",
        r.outcome
    );
    assert!(!r.budget_killed);
    assert_eq!(r.steps, 50);
}
