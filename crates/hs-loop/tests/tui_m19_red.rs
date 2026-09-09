//! REPL UI gap #10 M19 (found in the M17/M18 proof captures): the
//! transcript COMMITS the model's raw tool-call JSON as assistant
//! text. The protocol (lib.rs: `serde_json::from_str(&out.completion)`
//! over the WHOLE completion) makes a tool-call completion pure JSON,
//! and the delta channel streams every completion byte live, so the
//! raw envelope lands in scrollback next to the ▶/✓ beats that already
//! narrate the call. pi/omp never show the wire format.
//!
//! Contract: a call's streamed text is held in flight (still rendered
//! live - M11 streaming is untouched) until the call's disposition is
//! known. `ToolCallStart` DROPS the held text (the beat narrates it);
//! any other disposition commits it as before; mission end commits
//! whatever remains via `commit_answer_tail()`.

use hs_loop::tui::TuiState;
use hs_loop::uipaint::UiEvent;

const TOOL_JSON: &str = "{\"tool\":\"answer.write\",\"args\":{\"path\":\"/tmp/x\",\"content\":\"A\"}}\n";

fn model_end() -> UiEvent {
    UiEvent::ModelCallEnd { model: "m".into(), input_tokens: 1, output_tokens: 1 }
}
fn tool_start() -> UiEvent {
    UiEvent::ToolCallStart { plugin: "answer.write".into(), args_summary: "/tmp/x".into() }
}

fn transcript_text(st: &TuiState) -> String {
    st.transcript
        .iter()
        .map(|l| l.spans.iter().map(|s| s.content.to_string()).collect::<String>())
        .collect::<Vec<_>>()
        .join("\n")
}

// r1: a tool-call completion never reaches scrollback; the ▶ beat
// narrates the call instead.
#[test]
fn m19_tool_call_json_is_elided() {
    let mut st = TuiState::default();
    st.on_answer_delta(TOOL_JSON);
    st.on_ui_event(&model_end());
    st.on_ui_event(&tool_start());
    let t = transcript_text(&st);
    assert!(!t.contains("\"tool\""), "raw wire JSON elided, got: {t}");
    assert!(t.contains("answer.write"), "the ▶ beat narrates the call: {t}");
}

// r2: prose completions still commit - at the next event boundary.
#[test]
fn m19_prose_still_commits() {
    let mut st = TuiState::default();
    st.on_answer_delta("thinking out loud\n");
    st.on_ui_event(&model_end());
    assert!(
        !transcript_text(&st).contains("thinking out loud"),
        "held until disposition is known"
    );
    st.on_ui_event(&UiEvent::ModelCallStart { model: "m".into() });
    assert!(
        transcript_text(&st).contains("thinking out loud"),
        "prose commits at the next boundary"
    );
}

// r3: live streaming is preserved - the in-flight text is visible to
// the renderer BEFORE any commit decision.
#[test]
fn m19_live_tail_still_renders() {
    let mut st = TuiState::default();
    st.on_answer_delta("partial answer so");
    assert_eq!(st.answer_inflight, "partial answer so");
    assert!(!transcript_text(&st).contains("partial answer"), "not committed yet");
}

// r4: mission end commits whatever prose remains (budget-kill path:
// Done fires with text still held).
#[test]
fn m19_mission_end_commits_tail() {
    let mut st = TuiState::default();
    st.on_answer_delta("final prose without newline");
    st.on_ui_event(&model_end());
    st.commit_answer_tail();
    assert!(transcript_text(&st).contains("final prose without newline"));
    assert!(st.answer_inflight.is_empty());
}
