//! REPL UI gap #10 M23: the transcript clips every line longer than
//! the viewport. Zero wrapping in the render path: a prose answer, a
//! tool beat with long args, a goal echo, or a done line past the
//! right edge silently LOSES its tail - the reader cannot even tell
//! content is missing. pi/omp wrap transcript content. The composer
//! already grows for the editor; the transcript is the info-loss
//! surface.
//!
//! Contract: the transcript viewport word-wraps at the current width
//! (ratatui's own reflow, so rendering and scroll accounting never
//! diverge); scroll state is in visual ROWS (a pinned window stays
//! stable while wrapped content lands below); the "N below" marker
//! counts hidden rows; the streaming tail wraps too.

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

// R1: a prose answer longer than the width renders COMPLETELY -
// every word visible, on as many rows as it needs.
#[test]
fn r1_long_prose_wraps_fully_visible() {
    let mut st = TuiState::default();
    let theme = st.theme.clone();
    st.push_transcript_markdown(
        "alpha bravo charlie delta echo foxtrot golf hotel india juliet kilo lima ENDMARKER",
        &theme,
    );
    let text = viewport_text(&st, 40, 12);
    assert!(
        text.contains("ENDMARKER"),
        "the tail of a long line must be visible, not clipped: {text:?}"
    );
    assert!(text.contains("alpha"), "head visible: {text:?}");
}

// R2: scroll accounting is in ROWS - a line that wraps to two rows
// scrolls as two, and the marker counts hidden rows. ("x"*80 at
// width 40 reflows to exactly 2 rows: one hard-broken word.)
#[test]
fn r2_scroll_and_marker_count_rows() {
    let mut st = TuiState::default();
    let wide = "x".repeat(80);
    for _ in 0..20 {
        st.push_transcript_line(&wide); // 40 visual rows
    }
    st.transcript_scroll_up(3); // 3 rows up
    let text = viewport_text(&st, 40, 12);
    assert!(
        text.contains("3 below"),
        "marker counts hidden ROWS, not lines: {text:?}"
    );
    // Pinned-window stability: a new 2-row line landing while pinned
    // moves the hidden count by 2, and the top of the window must not
    // drift.
    let before = viewport_text(&st, 40, 12);
    st.push_transcript_line(&wide);
    let after = viewport_text(&st, 40, 12);
    assert!(after.contains("5 below"), "push adds rows: {after:?}");
    let top = |t: &str| t.lines().next().unwrap_or("").to_string();
    assert_eq!(
        top(&before),
        top(&after),
        "pinned window stays stable while wrapped content lands"
    );
}

// R3: a tool beat with long args wraps (the beat is where wire JSON
// used to clip - args past the edge were unreadable).
#[test]
fn r3_tool_beat_wraps() {
    let mut st = TuiState::default();
    st.push_transcript_line("\u{25b6} answer.write  {\"content\":\"abcdefghijklmnopqrstuvwxyz0123456789\",\"path\":\"/tmp/BEATMARK/answer.txt\"}");
    let text = viewport_text(&st, 40, 12);
    assert!(
        text.contains("BEATMARK"),
        "tool-beat args past the right edge must wrap into view: {text:?}"
    );
}

// R4: the streaming tail wraps while tail-following.
#[test]
fn r4_streaming_tail_wraps() {
    let mut st = TuiState::default();
    st.on_answer_delta("streaming words flow in one delta with no newline yet and keep coming TAILMARK");
    let text = viewport_text(&st, 40, 12);
    assert!(
        text.contains("TAILMARK"),
        "the in-flight tail wraps like committed text: {text:?}"
    );
}
