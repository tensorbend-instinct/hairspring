//! UI gap #10 M5: the loop rail goes LIVE - loop phase and the event
//! ticker derive from the same `UiEvent` stream the line-mode Painter
//! consumes, not from hardcoded state.

use hs_loop::tui::{LoopPhase, TuiState};
use hs_loop::uipaint::UiEvent;
use hs_core::EventKind;

fn model_start() -> UiEvent {
    UiEvent::ModelCallStart { model: "m".into() }
}
fn model_end() -> UiEvent {
    UiEvent::ModelCallEnd { model: "m".into(), input_tokens: 1, output_tokens: 1, cost_usd_micros: 100 }
}
fn tool_start() -> UiEvent {
    UiEvent::ToolCallStart { plugin: "term.exec".into(), args_summary: "ls".into() }
}
fn tool_end() -> UiEvent {
    UiEvent::ToolCallEnd { plugin: "term.exec".into(), ok: true, output_summary: "x".into(), elapsed_ms: 1 }
}

// R1: phase follows the event stream. M17 superseded the original
// mapping (boot at PLAN, ModelCallEnd forced REFLECT): boot and
// post-mission are IDLE, a call start after an observation is REFLECT,
// and the END of a call moves nothing.
#[test]
fn r1_phase_follows_events() {
    let mut st = TuiState::default();
    assert_eq!(st.phase, LoopPhase::Idle, "M17: nothing in flight at boot");
    st.on_ui_event(&model_start());
    assert_eq!(st.phase, LoopPhase::Plan);
    st.on_ui_event(&tool_start());
    assert_eq!(st.phase, LoopPhase::Act);
    st.on_ui_event(&tool_end());
    assert_eq!(st.phase, LoopPhase::Observe);
    st.on_ui_event(&model_end());
    assert_eq!(st.phase, LoopPhase::Observe, "M17: call end moves nothing");
    st.on_ui_event(&model_start());
    assert_eq!(st.phase, LoopPhase::Reflect, "M17: reasoning over the result");
}

// R2: the ticker records every event as its stream kind, in order.
#[test]
fn r2_ticker_records_kinds() {
    let mut st = TuiState::default();
    st.on_ui_event(&model_start());
    st.on_ui_event(&tool_start());
    st.on_ui_event(&tool_end());
    st.on_ui_event(&model_end());
    let kinds: Vec<EventKind> = st.ticker.iter().copied().collect();
    // One stream event per real event: the model call is ONE glyph
    // (start and end are UI beats of it), the tool result lands as an
    // Observation.
    assert_eq!(
        kinds,
        vec![EventKind::ModelCall, EventKind::ToolCall, EventKind::Observation],
        "one glyph per stream event"
    );
}

// R3: vitals flow too - model label, calls, and metered cost land on
// the HUD from events (cost via ModelCallEnd token counts at a rate).
#[test]
fn r3_vitals_from_events() {
    let mut st = TuiState::default();
    st.on_ui_event(&UiEvent::ModelCallStart { model: "deepseek-v4".into() });
    st.on_ui_event(&UiEvent::ModelCallEnd {
        model: "deepseek-v4".into(),
        input_tokens: 1000,
        output_tokens: 500,
        cost_usd_micros: 100,
    });
    assert_eq!(st.model_label, "deepseek-v4");
    assert_eq!(st.total_model_calls, 1);
    assert!(st.total_cost_micros > 0, "cost metered from tokens");
}

// R4: tool cards become transcript lines on the full-screen surface -
// the same beats the line-mode Painter prints, as transcript entries.
#[test]
fn r4_tool_beats_enter_transcript() {
    let mut st = TuiState::default();
    st.on_ui_event(&tool_start());
    st.on_ui_event(&tool_end());
    let text: String = st
        .transcript
        .iter()
        .map(|l| l.spans.iter().map(|s| s.content.clone()).collect::<String>())
        .collect::<Vec<_>>()
        .join("\n");
    assert!(text.contains("term.exec"), "plugin named: {text:?}");
    assert!(text.contains('\u{25b6}'), "start beat marker: {text:?}");
    assert!(text.contains('\u{2713}'), "ok beat marker: {text:?}");
}

// R5: markdown deltas accumulate in flight (rendered live from the
/// buffer) and commit at the DISPOSITION boundary (M19 superseded the
/// per-newline eager commit: a tool-call completion is pure wire JSON
/// and must never reach scrollback).
#[test]
fn r5_streaming_answer_commits_per_line() {
    let mut st = TuiState::default();
    st.on_answer_delta("## Hello");
    assert_eq!(st.transcript.len(), 0, "unterminated line stays in flight");
    st.on_answer_delta(" world\nsecond line\n");
    assert_eq!(st.transcript.len(), 0, "M19: held until disposition is known");
    st.commit_answer_tail();
    assert_eq!(st.transcript.len(), 2, "both lines commit at the boundary");
    let first: String = st.transcript[0].spans.iter().map(|s| s.content.clone()).collect();
    assert_eq!(first, "Hello world", "markdown header rendered on commit");
}
