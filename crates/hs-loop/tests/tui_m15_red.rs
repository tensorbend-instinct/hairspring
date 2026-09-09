//! UI gap #10 M15 RED: a scroll-pinned viewport must SAY it is
//! pinned. v3 cap4: after `PageUp` the frame showed old content with
//! no marker - indistinguishable from a stale screen, and the live
//! tail silently stopped tail-following. pi/omp always show a
//! scroll-state hint. Marker: bottom-right of the transcript
//! viewport, dim, "v N below" (N = lines hidden under the window).

use hs_loop::tui::{self, TuiState};
use ratatui::{backend::TestBackend, Terminal};

fn viewport_text(st: &TuiState, w: u16, h: u16) -> String {
    let backend = TestBackend::new(w, h);
    let mut term = Terminal::new(backend).unwrap();
    term.draw(|f| tui::render_skeleton(f, st)).unwrap();
    let buf = term.backend().buffer().clone();
    (0..h as usize)
        .map(|y| (0..w).map(|x| buf[(x, y as u16)].symbol().to_string()).collect::<String>())
        .collect::<Vec<_>>()
        .join("\n")
}

fn filled_state(lines: usize) -> TuiState {
    let mut st = TuiState::default();
    for i in 0..lines {
        st.push_transcript_line(&format!("line {i}"));
    }
    st
}

// R1: pinned 5 up shows the count of lines hidden below the window.
#[test]
fn r1_scrolled_marker_names_hidden_count() {
    let mut st = filled_state(40);
    st.transcript_scroll_up(5);
    let text = viewport_text(&st, 80, 24);
    assert!(text.contains("5 below"), "pinned viewport names the hidden tail: {text:?}");
}

// R2: tail-following shows no marker; scrolling back to the bottom
// removes it again.
#[test]
fn r2_marker_only_while_pinned() {
    let mut st = filled_state(40);
    assert!(
        !viewport_text(&st, 80, 24).contains("below"),
        "tail-follow has no marker"
    );
    st.transcript_scroll_up(3);
    assert!(viewport_text(&st, 80, 24).contains("3 below"));
    st.transcript_scroll_to_bottom();
    assert!(
        !viewport_text(&st, 80, 24).contains("below"),
        "back at the tail, marker gone"
    );
}
