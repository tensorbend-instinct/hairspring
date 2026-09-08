//! UI gap #10 M7: the event-loop seam. The full-screen surface turns
//! crossterm key events into actions through one pure function, so the
//! real terminal loop in the bin is thin glue and every keybinding is
//! unit-tested.

use hs_loop::tui::{self, KeyAction, TuiState};
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

fn key(c: char) -> KeyEvent {
    KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE)
}

// R1: plain typing edits the buffer; Enter submits it as an action.
#[test]
fn r1_typing_and_submit() {
    let mut st = TuiState::default();
    assert_eq!(tui::handle_key(&mut st, key('h')), KeyAction::Continue);
    tui::handle_key(&mut st, key('i'));
    match tui::handle_key(&mut st, KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)) {
        KeyAction::Submit(text) => assert_eq!(text, "hi"),
        other => panic!("expected Submit, got {other:?}"),
    }
    assert_eq!(st.editor.text(), "", "editor cleared after submit");
}

// R2: Alt+Enter inserts a newline (multi-line compose); Enter submits.
#[test]
fn r2_alt_enter_newline() {
    let mut st = TuiState::default();
    tui::handle_key(&mut st, key('a'));
    tui::handle_key(
        &mut st,
        KeyEvent::new(KeyCode::Enter, KeyModifiers::ALT),
    );
    tui::handle_key(&mut st, key('b'));
    assert_eq!(st.editor.text(), "a\nb");
}

// R3: Up/Down walk history when the editor sits on its first/last row;
/// Left/Right/Home/End move the cursor; Backspace deletes.
#[test]
fn r3_cursor_and_history_keys() {
    let mut st = TuiState::default();
    for c in "old".chars() {
        st.editor.input_char(c);
    }
    st.editor.submit();
    tui::handle_key(&mut st, KeyEvent::new(KeyCode::Up, KeyModifiers::NONE));
    assert_eq!(st.editor.text(), "old", "Up recalls history");
    tui::handle_key(&mut st, KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));
    assert_eq!(st.editor.text(), "", "Down returns to the live line");
    tui::handle_key(&mut st, key('x'));
    tui::handle_key(&mut st, key('y'));
    tui::handle_key(&mut st, KeyEvent::new(KeyCode::Left, KeyModifiers::NONE));
    tui::handle_key(&mut st, KeyEvent::new(KeyCode::Backspace, KeyModifiers::NONE));
    assert_eq!(st.editor.text(), "y");
}

// R4: PgUp/PgDn scroll the transcript; Ctrl+C quits; picker mode eats
// keys (Up/Down/Enter/Esc navigate the overlay, not the editor).
#[test]
fn r4_scroll_quit_and_picker_routing() {
    let mut st = TuiState::default();
    for i in 0..40 {
        st.push_transcript_line(&format!("l{i}"));
    }
    tui::handle_key(&mut st, KeyEvent::new(KeyCode::PageUp, KeyModifiers::NONE));
    assert!(st.transcript_scroll.is_some(), "PgUp pins scrollback");
    tui::handle_key(&mut st, KeyEvent::new(KeyCode::PageDown, KeyModifiers::NONE));
    assert!(st.transcript_scroll.is_none(), "PgDn back to the tail");
    assert_eq!(
        tui::handle_key(&mut st, KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL)),
        KeyAction::Quit
    );

    st.open_picker(vec!["one".into(), "two".into()]);
    tui::handle_key(&mut st, KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));
    assert_eq!(st.picker_selected(), Some(1), "picker owns Down while open");
    match tui::handle_key(&mut st, KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)) {
        KeyAction::Picked(choice) => assert_eq!(choice, "two"),
        other => panic!("expected Picked, got {other:?}"),
    }
    st.open_picker(vec!["x".into()]);
    assert_eq!(
        tui::handle_key(&mut st, KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)),
        KeyAction::Continue,
        "Esc cancels the picker"
    );
    assert!(st.picker_selected().is_none());
}

// R5: command submissions route to surface actions instead of
// missions: ":agents" toggles the panel, ":quit" quits, anything else
// is a mission/line command for the caller to dispatch.
#[test]
fn r5_command_routing() {
    let mut st = TuiState::default();
    for c in ":agents".chars() {
        tui::handle_key(&mut st, key(c));
    }
    assert_eq!(
        tui::handle_key(&mut st, KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)),
        KeyAction::ToggleAgents
    );
    for c in ":quit".chars() {
        tui::handle_key(&mut st, key(c));
    }
    assert_eq!(
        tui::handle_key(&mut st, KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)),
        KeyAction::Quit
    );
    for c in "fix the bug".chars() {
        tui::handle_key(&mut st, key(c));
    }
    match tui::handle_key(&mut st, KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)) {
        KeyAction::Submit(t) => assert_eq!(t, "fix the bug"),
        other => panic!("expected Submit, got {other:?}"),
    }
}
