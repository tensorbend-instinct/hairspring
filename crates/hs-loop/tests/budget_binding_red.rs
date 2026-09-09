//! D4 RED (Eric 2026-09-09, live DeepSeek-v4-pro burn): the REPL/TUI
//! session path never bound a budget - `budget_micros` stayed `None`
//! unless a `--budget-micros` flag was passed (only hs-swe-run/hs-tb-run
//! do that), so an interactive run with 0 missions closed burned $8.81
//! over 58 model calls with no in-process cap ever armed. THE LAW after
//! D4: every session binds a budget - from `[run] budget_usd` (or
//! `budget_micros`) in hairspring.toml when present, else the default
//! session cap ($10, the standing external guard figure). "Uncapped" is
//! never the silent default.
//!
//! Guards keep enforcing at the same per-step checkpoint in `run_mission`;
//! this defect was only the BINDING.

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
    let mut session = ReplSession::load(&config, log.path(), false, 50).unwrap();
    assert_eq!(
        session.budget_micros(),
        Some(5_000),
        "config [run] budget_usd must bind to the loop"
    );
    let r = session.run_goal("task-0").unwrap();
    assert!(r.budget_killed, "config-bound cap must kill the burn");
    assert!(r.model_calls <= 6, "killed near the cap, got {}", r.model_calls);
    assert!(session.total_cost_micros() <= 5_900);
}

#[test]
fn session_without_budget_config_still_binds_default_cap() {
    let dir = tempfile::tempdir().unwrap();
    let log = tempfile::tempdir().unwrap();
    let config = config_with_budget(dir.path(), "");
    let session = ReplSession::load(&config, log.path(), false, 4).unwrap();
    // No [run] stanza: still armed - the default session cap ($10).
    assert_eq!(
        session.budget_micros(),
        Some(10_000_000),
        "a session with no configured budget still binds the default cap"
    );
}

#[test]
fn explicit_set_budget_overrides_config() {
    let dir = tempfile::tempdir().unwrap();
    let log = tempfile::tempdir().unwrap();
    let config = config_with_budget(dir.path(), "[run]\nbudget_usd = 0.005");
    let mut session = ReplSession::load(&config, log.path(), false, 50).unwrap();
    session.set_budget_micros(42_000);
    assert_eq!(session.budget_micros(), Some(42_000));
}
