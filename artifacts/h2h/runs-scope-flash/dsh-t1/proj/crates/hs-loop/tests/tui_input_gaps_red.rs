//! M29 RED: TUI input gaps vs the Codex CLI / Claude Code bar, from
//! Eric's report: "pasting any text beyond one line with newlines
//! creates new missions - this is not working like codex or Claude
//! Code cli". Root cause: the terminal loop never enabled bracketed
//! paste (ESC[?2004h) and had no Event::Paste arm, so every pasted
//! newline arrived as an Enter key event and submitted its own
//! mission. The sweep also covers the editing-key surface those CLIs
//! provide: Ctrl+A/E/U/K/W, forward delete, word moves, and the
//! Ctrl+C / Ctrl+D semantics (interrupt/clear/quit, EOF/forward-del).

use hs_loop::tui::{self, KeyAction, LoopPhase, TuiState};
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

fn key(c: char) -> KeyEvent {
    KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE)
}

fn ctrl(c: char) -> KeyEvent {
    KeyEvent::new(KeyCode::Char(c), KeyModifiers::CONTROL)
}

fn alt(c: char) -> KeyEvent {
    KeyEvent::new(KeyCode::Char(c), KeyModifiers::ALT)
}

fn type_text(st: &mut TuiState, s: &str) {
    for c in s.chars() {
        tui::handle_key(st, key(c));
    }
}

// ---------- the reported defect: paste ----------

#[test]
fn paste_multiline_is_one_buffer_and_never_submits() {
    let mut st = TuiState::default();
    let action = tui::handle_paste(&mut st, "line one\nline two\nline three");
    assert_eq!(action, KeyAction::Continue, "a paste must never submit");
    assert_eq!(st.editor.text(), "line one\nline two\nline three");
    assert_eq!(st.editor.line_count(), 3);
    assert_eq!(st.editor.cursor(), (2, 10), "cursor lands at the end");
}

#[test]
fn paste_normalizes_crlf_and_lone_cr() {
    let mut st = TuiState::default();
    tui::handle_paste(&mut st, "a\r\nb\rc");
    assert_eq!(st.editor.text(), "a\nb\nc");
}

#[test]
fn paste_strips_control_chars_and_expands_tabs() {
    let mut st = TuiState::default();
    tui::handle_paste(&mut st, "a\u{7}b\tc\u{1}");
    assert_eq!(st.editor.text(), "ab  c");
}

#[test]
fn paste_then_enter_submits_the_whole_block_once() {
    let mut st = TuiState::default();
    tui::handle_paste(&mut st, "mission line 1\nmission line 2");
    let action = tui::handle_key(&mut st, KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    assert_eq!(
        action,
        KeyAction::Submit("mission line 1\nmission line 2".to_string()),
        "one Enter submits the pasted block as ONE mission"
    );
    assert_eq!(st.editor.history_entries().len(), 1, "one history entry, not two");
}

#[test]
fn paste_appends_into_existing_buffer() {
    let mut st = TuiState::default();
    type_text(&mut st, "prefix ");
    tui::handle_paste(&mut st, "pasted\ntext");
    assert_eq!(st.editor.text(), "prefix pasted\ntext");
}

// ---------- Ctrl+C / Ctrl+D semantics ----------

#[test]
fn ctrl_c_idle_empty_buffer_quits() {
    let mut st = TuiState::default();
    let action = tui::handle_key(&mut st, ctrl('c'));
    assert_eq!(action, KeyAction::Quit);
}

#[test]
fn ctrl_c_idle_nonempty_buffer_clears_instead_of_quitting() {
    let mut st = TuiState::default();
    type_text(&mut st, "half-typed goal");
    let action = tui::handle_key(&mut st, ctrl('c'));
    assert_eq!(action, KeyAction::Continue);
    assert_eq!(st.editor.text(), "");
}

#[test]
fn ctrl_c_mission_running_interrupts_and_keeps_the_buffer() {
    let mut st = TuiState::default();
    type_text(&mut st, "next goal draft");
    st.phase = LoopPhase::Act;
    let action = tui::handle_key(&mut st, ctrl('c'));
    assert_eq!(action, KeyAction::Interrupt, "mid-mission ^C interrupts, never kills the app");
    assert_eq!(st.editor.text(), "next goal draft", "the draft survives the interrupt");
}

#[test]
fn ctrl_d_empty_buffer_is_eof_quit() {
    let mut st = TuiState::default();
    let action = tui::handle_key(&mut st, ctrl('d'));
    assert_eq!(action, KeyAction::Quit);
}

#[test]
fn ctrl_d_nonempty_buffer_deletes_forward() {
    let mut st = TuiState::default();
    type_text(&mut st, "abc");
    tui::handle_key(&mut st, ctrl('a'));
    let action = tui::handle_key(&mut st, ctrl('d'));
    assert_eq!(action, KeyAction::Continue);
    assert_eq!(st.editor.text(), "bc");
    assert_eq!(st.editor.cursor(), (0, 0));
}

// ---------- editing keys ----------

#[test]
fn ctrl_a_and_ctrl_e_jump_line_ends() {
    let mut st = TuiState::default();
    type_text(&mut st, "hello");
    tui::handle_key(&mut st, ctrl('a'));
    assert_eq!(st.editor.cursor(), (0, 0));
    tui::handle_key(&mut st, ctrl('e'));
    assert_eq!(st.editor.cursor(), (0, 5));
}

#[test]
fn ctrl_u_kills_to_line_start() {
    let mut st = TuiState::default();
    type_text(&mut st, "hello world");
    for _ in 0..3 {
        tui::handle_key(&mut st, KeyEvent::new(KeyCode::Left, KeyModifiers::NONE));
    }
    tui::handle_key(&mut st, ctrl('u'));
    assert_eq!(st.editor.text(), "rld");
    assert_eq!(st.editor.cursor(), (0, 0));
}

#[test]
fn ctrl_k_kills_to_line_end() {
    let mut st = TuiState::default();
    type_text(&mut st, "hello world");
    tui::handle_key(&mut st, ctrl('a'));
    for _ in 0..5 {
        tui::handle_key(&mut st, KeyEvent::new(KeyCode::Right, KeyModifiers::NONE));
    }
    tui::handle_key(&mut st, ctrl('k'));
    assert_eq!(st.editor.text(), "hello");
    assert_eq!(st.editor.cursor(), (0, 5));
}

#[test]
fn ctrl_k_at_line_end_joins_the_next_line() {
    let mut st = TuiState::default();
    st.editor.set_text("ab\ncd");
    st.editor.move_up();
    st.editor.move_end();
    tui::handle_key(&mut st, ctrl('k'));
    assert_eq!(st.editor.text(), "abcd");
    assert_eq!(st.editor.line_count(), 1);
}

#[test]
fn ctrl_w_deletes_the_word_before_the_cursor() {
    let mut st = TuiState::default();
    type_text(&mut st, "hello world");
    tui::handle_key(&mut st, ctrl('w'));
    assert_eq!(st.editor.text(), "hello ");
    assert_eq!(st.editor.cursor(), (0, 6));
    tui::handle_key(&mut st, ctrl('w'));
    assert_eq!(st.editor.text(), "");
}

#[test]
fn word_moves_via_alt_arrows_alt_bf_and_ctrl_arrows() {
    let mut st = TuiState::default();
    type_text(&mut st, "foo bar baz");
    tui::handle_key(&mut st, KeyEvent::new(KeyCode::Left, KeyModifiers::ALT));
    assert_eq!(st.editor.cursor(), (0, 8), "alt+left lands on the word start");
    tui::handle_key(&mut st, alt('b'));
    assert_eq!(st.editor.cursor(), (0, 4), "alt+b walks another word left");
    tui::handle_key(&mut st, alt('f'));
    assert_eq!(st.editor.cursor(), (0, 8), "alt+f walks a word right");
    tui::handle_key(&mut st, KeyEvent::new(KeyCode::Right, KeyModifiers::CONTROL));
    assert_eq!(st.editor.cursor(), (0, 11), "ctrl+right lands at the end");
}

#[test]
fn word_moves_stay_line_local_in_a_multiline_buffer() {
    let mut st = TuiState::default();
    st.editor.set_text("foo\nbar baz");
    st.editor.move_up();
    st.editor.move_end();
    tui::handle_key(&mut st, KeyEvent::new(KeyCode::Right, KeyModifiers::ALT));
    assert_eq!(st.editor.cursor(), (0, 3), "no row hop off the end of a line");
    tui::handle_key(&mut st, KeyEvent::new(KeyCode::Left, KeyModifiers::ALT));
    assert_eq!(st.editor.cursor(), (0, 0), "alt+left from line end lands on its start");
}

#[test]
fn delete_key_deletes_forward() {
    let mut st = TuiState::default();
    type_text(&mut st, "ab");
    tui::handle_key(&mut st, ctrl('a'));
    tui::handle_key(&mut st, KeyEvent::new(KeyCode::Delete, KeyModifiers::NONE));
    assert_eq!(st.editor.text(), "b");
}

#[test]
fn history_walk_survives_the_new_editing_keys() {
    let mut st = TuiState::default();
    type_text(&mut st, "first goal");
    tui::handle_key(&mut st, KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    type_text(&mut st, "second goal");
    tui::handle_key(&mut st, KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    tui::handle_key(&mut st, KeyEvent::new(KeyCode::Up, KeyModifiers::NONE));
    assert_eq!(st.editor.text(), "second goal");
    tui::handle_key(&mut st, KeyEvent::new(KeyCode::Up, KeyModifiers::NONE));
    assert_eq!(st.editor.text(), "first goal");
    tui::handle_key(&mut st, KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));
    assert_eq!(st.editor.text(), "second goal");
}
