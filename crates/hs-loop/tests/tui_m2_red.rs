//! UI gap #10 M2: the composer editor - a real multi-line editing
//! surface inside the pinned box, replacing line-mode input on a TTY.
//!
//! Contract: an EditorState with cursor movement, insert/backspace,
//! newline (multi-line), submit-with-history, and rendering that puts
//! the terminal cursor on the editor's cursor cell.

use hs_loop::tui::{render_skeleton, TuiState};
use ratatui::{backend::{Backend, TestBackend}, Terminal};

// R1: typing inserts at the cursor and advances it.
#[test]
fn r1_typing_inserts_and_advances() {
    let mut st = TuiState::default();
    for c in "hello".chars() {
        st.editor.input_char(c);
    }
    assert_eq!(st.editor.text(), "hello");
    assert_eq!(st.editor.cursor(), (0, 5), "(row, col) after 5 chars");
    st.editor.move_left();
    st.editor.move_left();
    st.editor.input_char('X');
    assert_eq!(st.editor.text(), "helXlo");
}

// R2: backspace deletes left; at column 0 it joins with the line above.
#[test]
fn r2_backspace_and_newline() {
    let mut st = TuiState::default();
    for c in "ab".chars() {
        st.editor.input_char(c);
    }
    st.editor.insert_newline();
    for c in "cd".chars() {
        st.editor.input_char(c);
    }
    assert_eq!(st.editor.text(), "ab\ncd");
    assert_eq!(st.editor.cursor(), (1, 2));
    st.editor.move_home();
    st.editor.backspace(); // at (1,0): joins into "abcd"
    assert_eq!(st.editor.text(), "abcd");
    assert_eq!(st.editor.cursor(), (0, 2));
}

// R3: submit returns the text, clears the editor, and pushes history;
// history_up recalls the last entry, history_down walks back to empty.
#[test]
fn r3_submit_and_history() {
    let mut st = TuiState::default();
    for c in "first".chars() {
        st.editor.input_char(c);
    }
    assert_eq!(st.editor.submit(), Some("first".to_string()));
    assert_eq!(st.editor.text(), "");
    for c in "second".chars() {
        st.editor.input_char(c);
    }
    st.editor.submit();
    assert_eq!(st.editor.history_up(), Some("second".to_string()));
    assert_eq!(st.editor.text(), "second");
    assert_eq!(st.editor.history_up(), Some("first".to_string()));
    assert_eq!(st.editor.history_up(), Some("first".to_string()), "clamped at oldest");
    assert_eq!(st.editor.history_down(), Some("second".to_string()));
    assert_eq!(st.editor.history_down(), Some(String::new()), "back to the live line");
    assert_eq!(st.editor.submit(), None, "empty submit is None, no history entry");
}

// R4: the composer renders editor text and the frame cursor sits on the
// editor cursor cell (inside the box, after the "hs> " prompt).
#[test]
fn r4_render_places_cursor() {
    let backend = TestBackend::new(80, 24);
    let mut term = Terminal::new(backend).unwrap();
    let mut st = TuiState::default();
    for c in "probe".chars() {
        st.editor.input_char(c);
    }
    term.draw(|f| render_skeleton(f, &st)).unwrap();
    let buf = term.backend().buffer();
    let row: String = (0..80).map(|x| buf[(x, 21)].symbol()).collect();
    assert!(row.contains("hs> probe"), "editor text inside the box: {row:?}");
    let pos = term.backend_mut().get_cursor_position().unwrap();
    let (cx, cy) = (pos.x, pos.y);
    assert_eq!((cx, cy), (10, 21), "cursor after 'hs> probe' (1 border + 9 chars)");
}

// R5: vertical cursor movement across a multi-line buffer.
#[test]
fn r5_vertical_movement() {
    let mut st = TuiState::default();
    for c in "abc".chars() {
        st.editor.input_char(c);
    }
    st.editor.insert_newline();
    for c in "x".chars() {
        st.editor.input_char(c);
    }
    assert_eq!(st.editor.cursor(), (1, 1));
    st.editor.move_up();
    assert_eq!(st.editor.cursor(), (0, 1), "same column on the longer line");
    st.editor.move_end();
    st.editor.move_down();
    assert_eq!(st.editor.cursor(), (1, 1), "clamped at the shorter line's end");
}
