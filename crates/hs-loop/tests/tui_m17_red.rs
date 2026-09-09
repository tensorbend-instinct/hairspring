//! REPL UI gap #10 M17 (hostile-review item #5, 2026-09-08): the phase
//! rail mapping is coarse and STALE. Pre-M17: `ModelCallStart` always
//! meant PLAN and `ModelCallEnd` forced REFLECT, so the moment a mission
//! ended the rail kept glowing REFLECT forever - an idle composer
//! claiming the loop is reflecting. And the rail never showed the beat
//! that actually matters: reasoning over a fresh tool result.
//!
//! Contract (ReAct-honest semantics):
//! - No mission in flight: the rail is IDLE - no phase lit. A freshly
//!   started TUI and a just-finished mission both sit at Idle.
//! - `ModelCallStart` from Idle/Plan is PLAN (the operator deciding).
//! - `ToolCallStart` is ACT, `ToolCallEnd` is OBSERVE (unchanged).
//! - `ModelCallStart` when the previous beat was OBSERVE is REFLECT -
//!   the model reasoning over fresh observations.
//! - `ModelCallEnd` moves NOTHING: the end of a call is not a phase.

use hs_loop::tui::{LoopPhase, TuiState};
use hs_loop::uipaint::UiEvent;

fn model_start() -> UiEvent {
    UiEvent::ModelCallStart { model: "scripted".into() }
}
fn model_end() -> UiEvent {
    UiEvent::ModelCallEnd { model: "scripted".into(), input_tokens: 10, output_tokens: 5, cost_usd_micros: 100 }
}
fn tool_start() -> UiEvent {
    UiEvent::ToolCallStart { plugin: "term.exec".into(), args_summary: "ls".into() }
}
fn tool_end() -> UiEvent {
    UiEvent::ToolCallEnd { plugin: "term.exec".into(), ok: true, output_summary: "ok".into(), elapsed_ms: 3 }
}

// r1+r2: a fresh TUI is IDLE (nothing in flight), and the first model
// call of a mission lights PLAN.
#[test]
fn m17_idle_then_plan() {
    let mut st = TuiState::default();
    assert_eq!(st.phase, LoopPhase::Idle, "nothing in flight at boot");
    st.on_ui_event(&model_start());
    assert_eq!(st.phase, LoopPhase::Plan);
}

// r3: the act/observe beats are unchanged.
#[test]
fn m17_act_observe_beats() {
    let mut st = TuiState::default();
    st.on_ui_event(&model_start());
    st.on_ui_event(&tool_start());
    assert_eq!(st.phase, LoopPhase::Act);
    st.on_ui_event(&tool_end());
    assert_eq!(st.phase, LoopPhase::Observe);
}

// r4+r5: the model call that follows a tool result is REFLECT, and
// ModelCallEnd never moves the rail on its own.
#[test]
fn m17_reflect_on_observations() {
    let mut st = TuiState::default();
    st.on_ui_event(&model_start()); // Plan
    st.on_ui_event(&tool_start()); // Act
    st.on_ui_event(&tool_end()); // Observe
    st.on_ui_event(&model_start()); // reasoning over the observation
    assert_eq!(st.phase, LoopPhase::Reflect, "call after Observe reflects");
    st.on_ui_event(&model_end());
    assert_eq!(st.phase, LoopPhase::Reflect, "call end moves nothing");
}

// r6: mission completion returns the rail to IDLE - no stale REFLECT
// glowing over an idle composer.
#[test]
fn m17_done_returns_to_idle() {
    let mut st = TuiState::default();
    st.on_ui_event(&model_start());
    st.on_ui_event(&model_end());
    st.mission_done(1, 700);
    assert_eq!(st.phase, LoopPhase::Idle, "no stale phase after done");
}
