//! UI gap #10 M11 RED: mid-mission visibility polish from the live
//! captures. The streaming answer tail must RENDER while it is in
//! flight (M5 committed lines only, so a mission with no trailing
//! newline showed a blank viewport until completion), and multi-line
//! goal echoes must not swallow the newline.

use hs_loop::tui::{self, TuiState};
use ratatui::{backend::TestBackend, Terminal};

fn viewport_text(st: &TuiState, w: u16, h: u16) -> String {
    let backend = TestBackend::new(w, h);
    let mut term = Terminal::new(backend).unwrap();
    term.draw(|f| tui::render_skeleton(f, st)).unwrap();
    let buf = term.backend().buffer();
    (0..h as usize)
        .map(|y| (0..w).map(|x| buf[(x, y as u16)].symbol()).collect::<String>())
        .collect::<Vec<_>>()
        .join("\n")
}

// R1: an unterminated in-flight answer renders in the viewport as it
// streams - the surface is live DURING the call, not only after.
#[test]
fn r1_inflight_answer_renders_while_streaming() {
    let mut st = TuiState::default();
    st.on_answer_delta("partial answer still streaming");
    let text = viewport_text(&st, 80, 24);
    assert!(
        text.contains("partial answer still streaming"),
        "in-flight tail must be visible mid-stream: {text:?}"
    );
    assert_eq!(
        st.transcript.len(),
        0,
        "nothing committed without a newline or call boundary"
    );
}

// R2: a multi-line submitted goal echoes as multiple transcript
// lines, never one line with the newline swallowed.
#[test]
fn r2_multiline_echo_splits_lines() {
    let mut st = TuiState::default();
    st.push_goal_echo("fix the parser bug\nand keep the suite green");
    let joined: String = st
        .transcript
        .iter()
        .map(|l| l.spans.iter().map(|s| s.content.clone()).collect::<String>())
        .collect::<Vec<_>>()
        .join("\n");
    assert!(
        joined.contains("fix the parser bug\n"),
        "echo preserves the line break: {joined:?}"
    );
    assert!(joined.contains("and keep the suite green"));
}
