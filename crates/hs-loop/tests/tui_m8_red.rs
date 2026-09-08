//! UI gap #10 M8 RED: model-call boundary flush.
//!
//! Live-proof finding (tmux run, 25-step scripted mission): the delta
//! stream has NO trailing newline and repeats per model call, so the
//! in-flight answer buffer concatenated every call's prose into one
//! wrapped mega-line. The transcript must commit the in-flight answer
//! at each ModelCallEnd boundary: each call's prose lands as its own
//! markdown block, and the next call starts a fresh buffer.

use hs_loop::tui::TuiState;
use hs_loop::uipaint::UiEvent;

fn transcript_text(st: &TuiState) -> String {
    st.transcript
        .iter()
        .map(|l| l.spans.iter().map(|s| s.content.clone()).collect::<String>())
        .collect::<Vec<_>>()
        .join("\n")
}

// R1: an unterminated in-flight answer commits when the model call ends,
// leaving the buffer empty for the next call.
#[test]
fn r1_model_call_end_commits_inflight_answer() {
    let mut st = TuiState::default();
    st.on_answer_delta("partial answer without newline");
    st.on_ui_event(&UiEvent::ModelCallEnd {
        model: "m".into(),
        input_tokens: 10,
        output_tokens: 5,
    });
    assert!(
        st.answer_inflight.is_empty(),
        "ModelCallEnd must flush the in-flight answer, got {:?}",
        st.answer_inflight
    );
    assert!(
        transcript_text(&st).contains("partial answer without newline"),
        "committed transcript must contain the flushed answer"
    );
}

// R2: two calls' deltas commit as two separate transcript blocks -
// never concatenated into one line (the live-capture defect).
#[test]
fn r2_successive_calls_commit_separately() {
    let mut st = TuiState::default();
    st.on_answer_delta("first call prose");
    st.on_ui_event(&UiEvent::ModelCallEnd {
        model: "m".into(),
        input_tokens: 1,
        output_tokens: 1,
    });
    st.on_answer_delta("second call prose");
    st.on_ui_event(&UiEvent::ModelCallEnd {
        model: "m".into(),
        input_tokens: 1,
        output_tokens: 1,
    });
    let text = transcript_text(&st);
    let first = text.find("first call prose").expect("first block present");
    let second = text.find("second call prose").expect("second block present");
    assert!(first < second, "blocks commit in call order");
    // The concatenation defect would put both on ONE line.
    assert!(
        !st.transcript.iter().any(|l| {
            let s: String = l.spans.iter().map(|x| x.content.clone()).collect();
            s.contains("first call prose") && s.contains("second call prose")
        }),
        "two calls' prose must never share a transcript line"
    );
}

// R3: a call that streamed nothing (tool-call-only turn) commits
// nothing - no empty transcript lines from the boundary flush.
#[test]
fn r3_empty_inflight_commits_nothing() {
    let mut st = TuiState::default();
    let before = st.transcript.len();
    st.on_ui_event(&UiEvent::ModelCallEnd {
        model: "m".into(),
        input_tokens: 1,
        output_tokens: 1,
    });
    assert_eq!(
        st.transcript.len(),
        before,
        "empty in-flight buffer must not add transcript lines"
    );
}

// R4: markdown in the flushed block renders styled (same path as the
// newline commit): a heading flushes as a heading, not raw "## ...".
#[test]
fn r4_flushed_block_renders_markdown() {
    let mut st = TuiState::default();
    st.on_answer_delta("## Result\n\nfixed the parser");
    st.on_ui_event(&UiEvent::ModelCallEnd {
        model: "m".into(),
        input_tokens: 1,
        output_tokens: 1,
    });
    let text = transcript_text(&st);
    assert!(text.contains("Result"), "heading text present");
    assert!(
        !text.contains("## Result"),
        "heading must render styled, not raw hashes"
    );
    assert!(text.contains("fixed the parser"));
}
