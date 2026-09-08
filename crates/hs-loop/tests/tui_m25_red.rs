//! REPL UI gap #10 M25: the composer clips long input. The transcript
//! wraps (M23) but the composer renders the editor buffer as-is - a
//! goal longer than the box width loses its tail WHILE BEING TYPED,
//! and the cursor walks off the visible row. pi/omp wrap the input
//! box and grow it. Contract: composer content word-wraps at the box
//! inner width, the box grows by wrapped rows, and the cursor stays
//! on the cell that matches (row, col).

use hs_loop::tui::{self, TuiState};
use ratatui::{backend::{Backend, TestBackend}, Terminal};

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

// R1: a goal longer than the composer width is fully visible while
// typed - nothing clips at the box edge.
#[test]
fn r1_long_goal_wraps_in_composer() {
    let mut st = TuiState::default();
    for c in "refactor the tokenizer so the trailing fragment survives past GOALMARK".chars() {
        st.editor.input_char(c);
    }
    let text = screen(&st, 40, 12);
    assert!(
        text.contains("GOALMARK"),
        "the typed goal's tail must be visible in the composer: {text:?}"
    );
}

// R2: the box grows with the WRAPPED row count (a 70-char goal at
// width 40 needs more than the one hard line it is).
#[test]
fn r2_box_grows_by_wrapped_rows() {
    let mut st = TuiState::default();
    for c in "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".chars() {
        st.editor.input_char(c);
    }
    let text = screen(&st, 40, 12);
    // "hs> " + 70 a's = 74 cells at inner width 38 -> 2 rows minimum.
    let a_rows = text.lines().filter(|l| l.contains("aaaa")).count();
    assert!(a_rows >= 2, "composer wraps the buffer over rows: {text:?}");
}

// R3: cursor lands on the wrapped cell matching (row, col) - after
// typing past the wrap point the cursor is NOT off-screen or on the
// old unwrapped row.
#[test]
fn r3_cursor_follows_wrapped_position() {
    let mut st = TuiState::default();
    for c in "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".chars() {
        st.editor.input_char(c);
    }
    let backend = TestBackend::new(40, 12);
    let mut term = Terminal::new(backend).unwrap();
    term.draw(|f| tui::render_skeleton(f, &st)).unwrap();
    let pos = term.backend_mut().get_cursor_position().unwrap();
    let (cx, cy) = (pos.x, pos.y);
    // 74 cells at inner width 38: cursor sits at the end of row 1 of
    // the composer content, i.e. screen x = 1 + (74 - 38), y = box top + 2.
    assert!(cx > 1, "cursor x inside the box: ({cx},{cy})");
    assert!(
        cy >= 8,
        "cursor on a LATER row than an unwrapped render would give: ({cx},{cy})"
    );
}
