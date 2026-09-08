//! REPL UI gap #10 M27: the loop rail truncates its own phases at
//! narrow widths. Live probe at 40 columns: the rail read
//! "PLAN > ACT > OBSERVE > R" - REFLECT clipped mid-word, because the
//! phases get a fixed 3/5 of the width (24 cells at 40) while the
//! labels need 30. The rail is the loop's identity; clipping its
//! names is toy-grade. (The wide-emoji ticker worry from the queue
//! was measured and is a NON-bug: ratatui right-aligns by cells.)
//!
//! Contract: phases always get their full measured width; the ticker
//! takes the remainder and yields entirely when the remainder is too
//! small to read; only a terminal narrower than the rail itself clips
//! phase names.

use hs_loop::tui::{self, TuiState};
use ratatui::{backend::TestBackend, Terminal};

fn rail_row(st: &TuiState, w: u16, h: u16) -> String {
    let backend = TestBackend::new(w, h);
    let mut term = Terminal::new(backend).unwrap();
    term.draw(|f| tui::render_skeleton(f, st)).unwrap();
    let buf = term.backend().buffer().clone();
    (0..h)
        .map(|y| (0..w).map(|x| buf[(x, y)].symbol().to_string()).collect::<String>())
        .find(|row| row.contains("PLAN"))
        .expect("rail row")
}

// R1: at 40 columns the full rail - including REFLECT - is visible,
// with the ticker still on the right.
#[test]
fn r1_full_phases_at_40_cols() {
    let mut st = TuiState::default();
    st.push_stream_event(hs_core::EventKind::ModelCall);
    let row = rail_row(&st, 40, 12);
    assert!(row.contains("REFLECT"), "no clipped phases: {row:?}");
    assert!(row.contains("\u{25c6}"), "ticker survives: {row:?}");
}

// R2: at 34 columns the phases still fit whole; the ticker keeps a
// readable minimum.
#[test]
fn r2_phases_win_over_ticker() {
    let mut st = TuiState::default();
    st.push_stream_event(hs_core::EventKind::ModelCall);
    st.push_stream_event(hs_core::EventKind::ToolCall);
    let row = rail_row(&st, 34, 12);
    assert!(row.contains("REFLECT"), "phases whole at 34: {row:?}");
}

// R3: narrower than the rail itself - phases clip (degenerate), but
// nothing panics and the ticker yields completely.
#[test]
fn r3_degenerate_width_yields_ticker() {
    let mut st = TuiState::default();
    st.push_stream_event(hs_core::EventKind::ModelCall);
    let row = rail_row(&st, 24, 12);
    assert!(row.contains("PLAN"), "rail still renders: {row:?}");
    assert!(
        !row.contains("\u{25c6}"),
        "ticker yields when it cannot be readable: {row:?}"
    );
}
