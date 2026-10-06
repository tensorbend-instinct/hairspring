//! RED (Eric 2026-10-06, usability gap list): raw reasoning dumped up to
//! 8 lines per call, tool rows printed "ok 252ms" plus output, and a long
//! silent tool looked like a hang. Reasoning folds to one row with a
//! line count and expands on demand; a finished tool is one
//! "Completed in Ns" row; a running tool shows a live "<tool> Ns" ticker.

use hs_loop::tui::TuiState;
use hs_loop::uipaint::UiEvent as U;
use std::time::{Duration, Instant};

fn flat(st: &TuiState) -> String {
    st.transcript_spans_tail().iter().map(|(t, _)| t.clone()).collect()
}

#[test]
fn long_reasoning_folds_to_one_row_with_line_count() {
    let mut st = TuiState::default();
    let text = (1..=6).map(|i| format!("thought line {i}")).collect::<Vec<_>>().join("\n");
    st.on_ui_event(&U::ModelReasoning { text });
    let f = flat(&st);
    assert!(f.contains("thought line 1"), "first line stays visible: {f}");
    assert!(f.contains("+5 more lines"), "fold marker with count: {f}");
    assert!(!f.contains("thought line 4"), "body folded: {f}");
}

#[test]
fn short_reasoning_is_shown_whole() {
    let mut st = TuiState::default();
    st.on_ui_event(&U::ModelReasoning { text: "just one thought".into() });
    let f = flat(&st);
    assert!(f.contains("just one thought") && !f.contains("more line"), "{f}");
}

#[test]
fn reasoning_expands_on_toggle_and_keeps_full_text() {
    let mut st = TuiState::default();
    let text = (1..=6).map(|i| format!("thought line {i}")).collect::<Vec<_>>().join("\n");
    st.on_ui_event(&U::ModelReasoning { text });
    assert_eq!(st.last_reasoning().lines().count(), 6, "full text kept");
    st.set_reasoning_expanded(true);
    st.on_ui_event(&U::ModelReasoning { text: "a\nb\nc\nd".into() });
    let f = flat(&st);
    assert!(f.contains('d') && !f.contains("more line"), "expanded shows all: {f}");
}

#[test]
fn finished_tool_is_one_completed_in_row() {
    let mut st = TuiState::default();
    st.on_ui_event(&U::ToolCallStart { plugin: "term.exec".into(), args_summary: "pytest".into() });
    st.on_ui_event(&U::ToolCallEnd {
        plugin: "term.exec".into(),
        ok: true,
        output_summary: "6 passed".into(),
        elapsed_ms: 2400,
    });
    let f = flat(&st);
    assert!(f.contains("Completed in 2.4s"), "{f}");
    let st2 = {
        let mut s = TuiState::default();
        s.on_ui_event(&U::ToolCallEnd {
            plugin: "t".into(),
            ok: false,
            output_summary: String::new(),
            elapsed_ms: 250,
        });
        s
    };
    let g = flat(&st2);
    assert!(g.contains("Failed in 250ms"), "{g}");
}

#[test]
fn running_tool_shows_live_ticker_and_clears_when_done() {
    let mut st = TuiState::default();
    let t0 = Instant::now();
    st.on_ui_event_at(&U::ToolCallStart { plugin: "term.exec".into(), args_summary: "pytest".into() }, t0);
    let line = st.footer_stats(t0 + Duration::from_secs(12));
    assert!(line.contains("term.exec 12s"), "ticker: {line}");
    st.on_ui_event_at(
        &U::ToolCallEnd { plugin: "term.exec".into(), ok: true, output_summary: String::new(), elapsed_ms: 12000 },
        t0 + Duration::from_secs(12),
    );
    let after = st.footer_stats(t0 + Duration::from_secs(20));
    assert!(!after.contains("term.exec 20s"), "ticker cleared: {after}");
}

#[test]
fn reasoning_command_toggles_and_shows_full_text() {
    let mut st = TuiState::default();
    let text = (1..=5).map(|i| format!("L{i}")).collect::<Vec<_>>().join("\n");
    st.on_ui_event(&U::ModelReasoning { text });
    assert!(st.handle_reasoning_command("/reasoning show"));
    let f = flat(&st);
    assert!(f.contains("L5"), "full text printed: {f}");
    assert!(st.handle_reasoning_command("/reasoning"));
    assert!(flat(&st).contains("reasoning: expanded"));
    assert!(!st.handle_reasoning_command("/other"));
}
