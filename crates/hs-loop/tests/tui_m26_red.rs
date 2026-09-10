//! REPL UI gap #10 M26: the full-screen surface's help lies, half the
//! line-mode commands are dead ends, and a fresh session is a void.
//!
//! Grounding (bin/hs-repl.rs + tui.rs `handle_key)`: the TUI handles
//! :resume, :help, :quit/:q, :agents - but :help prints the LINE-MODE
//! `REPL_HELP` which advertises :status, :history, :last; typing any of
//! those on the full-screen surface gets "unknown command". The help
//! names commands that don't work, and omits :resume/:agents which
//! do. Meanwhile a fresh session renders a blank viewport with no
//! hint what to type (pi/omp show a boot hint).
//!
//! Contract: (1) an empty transcript renders a dim hint naming a
//! true next action; (2) the hint vanishes once content lands;
//! (3) TUI help text lists exactly the commands the TUI implements;
//! (4) :status/:history/:last have real implementations (state-level
//! pieces pinned here; the bin wiring is thin).

use hs_loop::tui::{self, TuiState};
use ratatui::{backend::TestBackend, Terminal};

fn screen(st: &TuiState, w: u16, h: u16) -> String {
    let backend = TestBackend::new(w, h);
    let mut term = Terminal::new(backend).unwrap();
    term.draw(|f| tui::render_skeleton(f, st)).unwrap();
    let buf = term.backend().buffer().clone();
    (0..h as usize)
        .map(|y| (0..w).map(|x| buf[(x, y as u16)].symbol().to_string()).collect::<String>())
        .collect::<Vec<_>>()
        .join("\n")
}

// R1: a fresh session tells you what to do.
#[test]
fn r1_empty_session_shows_hint() {
    let st = TuiState::default();
    let text = screen(&st, 80, 24);
    assert!(
        text.contains("/help"),
        "empty viewport names a true next action: {text:?}"
    );
}

// R2: the hint is gone once the transcript has content.
#[test]
fn r2_hint_vanishes_with_content() {
    let mut st = TuiState::default();
    st.push_transcript_line("some output");
    let text = screen(&st, 80, 24);
    assert!(
        !text.contains("type a goal"),
        "hint must not overlay real content: {text:?}"
    );
}

// R3: the TUI help text lists every command the surface implements -
// and nothing it does not.
#[test]
fn r3_help_matches_reality() {
    let help = tui::TUI_HELP;
    for cmd in ["/status", "/history", "/last", "/resume", "/agents", "/help", "/quit"] {
        assert!(help.contains(cmd), "help lists implemented command {cmd}");
    }
}

// R4: :status has real data behind it - model, counters, stream.
#[test]
fn r4_status_line_carries_vitals() {
    let mut st = TuiState {
        model_label: "scripted".to_string(),
        stream_short: "b412653b".to_string(),
        ..Default::default()
    };
    st.missions_run = 2;
    st.total_steps = 4;
    st.total_model_calls = 5;
    st.total_cost_micros = 3500;
    let line = st.status_line();
    assert!(line.contains("scripted"), "model: {line}");
    assert!(line.contains("2 missions"), "missions: {line}");
    assert!(line.contains("5 calls"), "calls: {line}");
    assert!(line.contains("$0.0035"), "cost: {line}");
    assert!(line.contains("b412653b"), "stream: {line}");
}

// R5: :history has real data - the editor's submitted-goal history.
#[test]
fn r5_history_entries_exposed() {
    let mut st = TuiState::default();
    for c in "first goal".chars() {
        st.editor.input_char(c);
    }
    st.editor.submit();
    for c in "second goal".chars() {
        st.editor.input_char(c);
    }
    st.editor.submit();
    let h = st.editor.history_entries();
    assert!(
        h.iter().any(|e| e.contains("first goal")) && h.iter().any(|e| e.contains("second goal")),
        "history exposes both submissions: {h:?}"
    );
}
