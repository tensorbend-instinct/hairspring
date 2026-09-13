//! D4 RED (Eric 2026-09-09, live DeepSeek-v4-pro burn): the REPL/TUI
//! session path never bound a budget - `budget_micros` stayed `None`
//! unless a `--budget-micros` flag was passed (only hs-swe-run/hs-tb-run
//! do that), so an interactive run with 0 missions closed burned $8.81
//! over 58 model calls with no in-process cap ever armed. THE LAW after
//! D4: every session binds a budget - from `[run] budget_usd` (or
//! `budget_micros`) in hairspring.toml when present, else the default
//! session cap ($10, the standing external guard figure).
//!
//! 2026-09-12: Eric overruled the silent default - caps are opt-in
//! (no [run] stanza = no cap); the binding machinery and the guard
//! semantics are unchanged. Guards keep enforcing at the same per-step
//! checkpoint in `run_mission`.

use hs_loop::repl::ReplSession;

const BENCHMODEL: &str = env!("CARGO_BIN_EXE_hs-plugin-benchmodel");
const ANSWER: &str = env!("CARGO_BIN_EXE_hs-plugin-answer");
const CHECKER: &str = env!("CARGO_BIN_EXE_hs-plugin-checker");

fn config_with_budget(dir: &std::path::Path, extra: &str) -> std::path::PathBuf {
    let p = dir.join("hairspring.toml");
    std::fs::write(
        &p,
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
subjects = ["*"]

{extra}
"#
        ),
    )
    .unwrap();
    p
}

#[test]
fn session_binds_budget_from_config_run_stanza() {
    let dir = tempfile::tempdir().unwrap();
    let log = tempfile::tempdir().unwrap();
    // benchmodel reports 900 micro-USD per call and never repairs: a
    // $0.005 config cap must kill after ~5 calls.
    let config = config_with_budget(dir.path(), "[run]\nbudget_usd = 0.005");
    let mut session = ReplSession::load(&config, log.path(), false, Some(50)).unwrap();
    assert_eq!(
        session.budget_micros(),
        Some(5_000),
        "config [run] budget_usd must bind to the loop"
    );
    // Burn vehicle: task-20 is UNREPAIRABLE by design (the 24-task
    // fixture family: 0..18 feedback-repairable, 18..24 not). Since dance
    // #95 arms mission memory for every run_goal, benchmodel now REPAIRS
    // task-0 from the injected checker feedback and the mission converges
    // - the correct behavior - so the runaway-burn pin needs a task that
    // can never go green.
    let r = session.run_goal("task-20").unwrap();
    assert!(r.budget_killed, "config-bound cap must kill the burn: {r:?}");
    assert!(r.model_calls <= 6, "killed near the cap, got {}", r.model_calls);
    assert!(session.total_cost_micros() <= 5_900);
}

/// Eric 2026-09-12 SUPERSEDED the D4 default: "the caps should start
/// with no caps... I set 200 steps and 100 dollars on a task and it
/// stopped at a cap of 10 dollars which was strange." The hidden $10 is
/// gone - a session with no configured budget binds NO cap; budgets are
/// opt-in via [run] `budget_usd`, `--budget-micros`, or /caps budget.
#[test]
fn session_without_budget_config_binds_no_cap() {
    let dir = tempfile::tempdir().unwrap();
    let log = tempfile::tempdir().unwrap();
    let config = config_with_budget(dir.path(), "");
    let session = ReplSession::load(&config, log.path(), false, Some(4)).unwrap();
    assert_eq!(
        session.budget_micros(),
        None,
        "no configured budget = no cap armed (Eric 2026-09-12)"
    );
}

#[test]
fn explicit_set_budget_overrides_config() {
    let dir = tempfile::tempdir().unwrap();
    let log = tempfile::tempdir().unwrap();
    let config = config_with_budget(dir.path(), "[run]\nbudget_usd = 0.005");
    let mut session = ReplSession::load(&config, log.path(), false, Some(50)).unwrap();
    session.set_budget_micros(42_000);
    assert_eq!(session.budget_micros(), Some(42_000));
}
