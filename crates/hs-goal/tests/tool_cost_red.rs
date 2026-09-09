//! RED (hostile review, 2026-09-09): hs-loop records every call's cost in
//! the event's canonical `cost_usd_micros` field, but hs-goal's `OuterLoop`
//! records `ToolCall` events with cost 0 - the tool's reported cost survives
//! only inside the payload body. The crate doc says budgets read cost from
//! the log's own records; a reconstruction from canonical fields would
//! undercount tool spend to zero.
//!
//! Falsifier: run a one-tool mission whose tool reports
//! `cost_usd_micros` = 1234; the `ToolCall` event's canonical cost field
//! must equal 1234.

use hs_goal::*;

const COSTTOOL: &str = env!("CARGO_BIN_EXE_hs-plugin-costtool");
const COSTMODEL: &str = env!("CARGO_BIN_EXE_hs-plugin-costmodel");

#[test]
fn tool_call_events_carry_their_cost_in_the_canonical_field() {
    let dir = tempfile::tempdir().unwrap();
    let log = tempfile::tempdir().unwrap();
    let config = dir.path().join("hairspring.toml");
    std::fs::write(
        &config,
        format!(
            r#"
[[tools]]
name = "costtool.probe"
command = ["{COSTTOOL}"]
subjects = ["*"]

[[models]]
name = "costmodel"
command = ["{COSTMODEL}"]
default = true
"#
        ),
    )
    .unwrap();
    let kernel = hs_kernel::Kernel::load(&config).unwrap();
    let mut l = OuterLoop::new(kernel, log.path(), 0).unwrap();
    let g = Goal::new(
        "cost-probe",
        CompletionMode::SelfDeclared,
        Budget {
            max_steps: 4,
            max_cost_usd_micros: 1_000_000,
        },
    );
    let out = l.run(&g).unwrap();
    assert!(
        matches!(out, MissionOutcome::Passed { .. }),
        "mission should pass by say-so: {out:?}"
    );
    let reader = hs_log::StreamReader::open(log.path(), l.stream_id()).unwrap();
    let events = reader.events().unwrap();
    let tool_costs: Vec<i64> = events
        .iter()
        .filter(|e| e.kind == hs_core::EventKind::ToolCall)
        .map(|e| e.cost_usd_micros)
        .collect();
    assert_eq!(
        tool_costs,
        vec![1234],
        "tool call cost must land in the canonical field"
    );
}
