//! D5 RED (Eric 2026-09-09 postmortem, parent 10:54 add): the loop books
//! ONE cost figure per model call - usage tokens x list rates WITH cache
//! credit applied. A guard must never trust the discounted figure as its
//! ceiling: cache pricing is the provider's to change. THE LAW after D5:
//! every model call books BOTH the provider-reported figure (cache
//! credit honored, as billed today) AND the conservative list-rate
//! figure (every input token at the full in-rate, every output token at
//! the full out-rate - no cache credit). Both land in the ledger
//! (`ModelCall` events) and in run/session reporting; the budget guard
//! binds the CONSERVATIVE counter. A plugin that reports no conservative
//! figure books the provider figure as both (fixtures, legacy plugins).
//!
//! 2026-09-12 amendment (Eric, live mission c91e8de3): conservative is
//! the DEFAULT for direct InnerLoop users (the benchmark binaries).
//! User-facing sessions (hs-repl one-shot + TUI) arm
//! BudgetGuardMode::ProviderReported - a user's dollar cap binds the
//! billed figure (see budget_guard_mode_red.rs).
//!
//! Fixture reality: benchmodel reports `cost_usd_micros`=900 (provider)
//! and `conservative_cost_usd_micros`=1200 per call.

use hs_loop::*;

const ANSWER: &str = env!("CARGO_BIN_EXE_hs-plugin-answer");
const CHECKER: &str = env!("CARGO_BIN_EXE_hs-plugin-checker");
const BENCHMODEL: &str = env!("CARGO_BIN_EXE_hs-plugin-benchmodel");

fn config(dir: &std::path::Path) -> std::path::PathBuf {
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
"#
        ),
    )
    .unwrap();
    p
}

#[test]
fn guard_binds_conservative_and_both_figures_book() {
    let dir = tempfile::tempdir().unwrap();
    let log = tempfile::tempdir().unwrap();
    let kernel = hs_kernel::Kernel::load(&config(dir.path())).unwrap();
    let mut l = InnerLoop::new(kernel, log.path(), false, 50).unwrap();
    l.set_budget_micros(5_000);
    let r = l.run_mission("task-0").unwrap();
    assert!(r.budget_killed, "conservative counter outruns the cap");
    // Conservative 1200/call: kill at call 5 (6000 > 5000). The
    // provider figure alone (5x900 = 4500) would NOT have killed.
    assert_eq!(r.model_calls, 5, "guard fired on the provider figure");
    assert_eq!(l.total_cost_micros(), 4_500, "provider-reported counter");
    assert_eq!(
        l.conservative_cost_total_micros(),
        6_000,
        "conservative list-rate counter"
    );
    assert_eq!(r.cost_micros, 4_500);
    assert_eq!(r.conservative_cost_micros, 6_000);
}

#[test]
fn ledger_model_call_events_book_both_figures() {
    let dir = tempfile::tempdir().unwrap();
    let log = tempfile::tempdir().unwrap();
    let kernel = hs_kernel::Kernel::load(&config(dir.path())).unwrap();
    let mut l = InnerLoop::new(kernel, log.path(), false, 3).unwrap();
    l.set_budget_micros(1_000_000);
    let r = l.run_mission("task-0").unwrap();
    let reader = hs_log::StreamReader::open(log.path(), r.stream_id).unwrap();
    let mut calls = 0u64;
    for e in reader.events().unwrap() {
        if e.kind != hs_core::EventKind::ModelCall {
            continue;
        }
        let b = reader.resolve_payload(&e).unwrap();
        let v: serde_json::Value = serde_json::from_slice(&b).unwrap();
        assert_eq!(v["cost_usd_micros"].as_u64(), Some(900), "provider figure");
        assert_eq!(
            v["conservative_cost_usd_micros"].as_u64(),
            Some(1200),
            "conservative figure booked alongside"
        );
        calls += 1;
    }
    assert!(calls >= 2, "mission made model calls, got {calls}");
}
