//! 2026-09-12 RED (Eric, live TUI mission c91e8de3): he armed a $40
//! session budget and the mission died `budget_killed` at $3.4416 of
//! REAL spend - the guard bound the conservative list-rate counter
//! (~12x real at 92-98% DeepSeek cache hit, on 5x-flash assumed v4-pro
//! rates). That D5 law is benchmark accounting, not user money: to the
//! person typing "/caps budget 40" the cap means dollars actually
//! billed. THE LAW now: the budget guard mode is explicit. User-facing
//! sessions (hs-repl one-shot + TUI) bind the PROVIDER-REPORTED
//! counter; benchmark binaries (hs-swe-run / hs-tb-run, direct
//! InnerLoop users) keep the conservative list-rate counter for
//! cross-arm ledger comparability. The kill event records which
//! counter fired.
//!
//! Fixture reality: benchmodel reports `cost_usd_micros`=900 (provider)
//! and `conservative_cost_usd_micros`=1200 per call.

use hs_loop::repl::ReplSession;
use hs_loop::*;

const ANSWER: &str = env!("CARGO_BIN_EXE_hs-plugin-answer");
const CHECKER: &str = env!("CARGO_BIN_EXE_hs-plugin-checker");
const BENCHMODEL: &str = env!("CARGO_BIN_EXE_hs-plugin-benchmodel");

fn config(dir: &std::path::Path, extra: &str) -> std::path::PathBuf {
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
fn provider_mode_spares_mission_whose_real_spend_is_under_cap() {
    let dir = tempfile::tempdir().unwrap();
    let log = tempfile::tempdir().unwrap();
    let kernel = hs_kernel::Kernel::load(&config(dir.path(), "")).unwrap();
    let mut l = InnerLoop::new(kernel, log.path(), false, 5).unwrap();
    l.set_budget_guard_mode(BudgetGuardMode::ProviderReported);
    l.set_budget_micros(5_000);
    let r = l.run_mission("task-0").unwrap();
    // 5 calls x 900 = 4500 provider (< 5000 cap) but 6000 conservative
    // (> cap): the conservative guard would kill at call 5, the
    // provider guard lets the mission run its steps out.
    assert!(
        !r.budget_killed,
        "real spend 4500 is under the 5000 cap: {r:?}"
    );
    assert_eq!(r.model_calls, 5, "mission ran its steps out");
}

#[test]
fn default_mode_stays_conservative_for_benchmark_arms() {
    let dir = tempfile::tempdir().unwrap();
    let log = tempfile::tempdir().unwrap();
    let kernel = hs_kernel::Kernel::load(&config(dir.path(), "")).unwrap();
    let mut l = InnerLoop::new(kernel, log.path(), false, 50).unwrap();
    l.set_budget_micros(5_000);
    let r = l.run_mission("task-0").unwrap();
    assert!(r.budget_killed, "default guard stays conservative");
    assert_eq!(r.model_calls, 5, "conservative 1200/call kills at 5");
}

#[test]
fn repl_sessions_arm_provider_reported_guard() {
    let dir = tempfile::tempdir().unwrap();
    let log = tempfile::tempdir().unwrap();
    let config = config(dir.path(), "[run]\nbudget_usd = 0.005");
    let mut session = ReplSession::load(&config, log.path(), false, Some(50)).unwrap();
    assert_eq!(
        session.budget_guard_mode(),
        BudgetGuardMode::ProviderReported,
        "user-facing sessions guard real dollars"
    );
    // Burn vehicle: task-20 is unrepairable by design. Provider
    // 900/call kills at call 6 (5400 > 5000); conservative would have
    // killed at call 5.
    let r = session.run_goal("task-20").unwrap();
    assert!(r.budget_killed, "the cap still kills a real burn: {r:?}");
    assert_eq!(r.model_calls, 6, "guard fired on real spend");
    assert_eq!(session.total_cost_micros(), 5_400);
}

#[test]
fn kill_event_records_the_guard_that_fired() {
    let dir = tempfile::tempdir().unwrap();
    let log = tempfile::tempdir().unwrap();
    let kernel = hs_kernel::Kernel::load(&config(dir.path(), "")).unwrap();
    let mut l = InnerLoop::new(kernel, log.path(), false, 50).unwrap();
    l.set_budget_micros(5_000);
    let r = l.run_mission("task-0").unwrap();
    assert!(r.budget_killed);
    let reader = hs_log::StreamReader::open(log.path(), r.stream_id).unwrap();
    let mut found = false;
    for e in reader.events().unwrap() {
        if e.kind != hs_core::EventKind::Feedback {
            continue;
        }
        let b = reader.resolve_payload(&e).unwrap();
        let v: serde_json::Value = serde_json::from_slice(&b).unwrap();
        if v["budget_killed"].as_bool() == Some(true) {
            assert_eq!(
                v["guard"].as_str(),
                Some("conservative"),
                "kill event names the counter that fired"
            );
            assert!(v["cost_micros"].as_u64().unwrap() < v["conservative_cost_micros"].as_u64().unwrap());
            found = true;
        }
    }
    assert!(found, "budget_killed event present in the stream");
}
