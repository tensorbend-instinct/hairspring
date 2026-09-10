//! Command palette RED (Eric 2026-09-10: "typing into the TUI bar with
//! the command does not show the command options ... commands in common
//! TUIs are /<command> with immediate feedback"). Canonical commands are
//! /<command>; :<command> stays a backward-compatible alias. While the
//! editor buffer starts with a sigil, a live palette lists matching
//! commands with descriptions; Up/Down navigate, Enter/Tab accept, Esc
//! closes, and an unknown command draws immediate inline feedback - it
//! never becomes a mission.

use hs_loop::tui::{self, KeyAction, TuiState};
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

fn type_str(st: &mut TuiState, s: &str) {
    for c in s.chars() {
        tui::handle_key(st, KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE));
    }
}

fn enter() -> KeyEvent {
    KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)
}

// P1: a bare sigil opens the palette with the FULL command set.
#[test]
fn p1_slash_opens_palette_with_all_commands() {
    let mut st = TuiState::default();
    assert!(st.palette_matches().is_none(), "no palette before sigil");
    type_str(&mut st, "/");
    let m = st.palette_matches().expect("palette opens on /");
    assert!(
        m.len() >= 10,
        "every command discoverable from a bare /: {m:?}"
    );
    assert!(m.contains(&"help"), "help listed: {m:?}");
    assert!(m.contains(&"agents"), "agents listed: {m:?}");
}

// P2: a partial name filters live.
#[test]
fn p2_partial_name_filters() {
    let mut st = TuiState::default();
    type_str(&mut st, "/ag");
    let m = st.palette_matches().expect("palette open");
    assert_eq!(m, vec!["agents"], "typed prefix filters: {m:?}");
}

// P3: Up/Down move the palette selection, not the editor history.
#[test]
fn p3_up_down_navigate_palette() {
    let mut st = TuiState::default();
    type_str(&mut st, "/");
    assert_eq!(st.palette_selected(), Some(0));
    tui::handle_key(&mut st, KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));
    assert_eq!(st.palette_selected(), Some(1));
    tui::handle_key(&mut st, KeyEvent::new(KeyCode::Up, KeyModifiers::NONE));
    assert_eq!(st.palette_selected(), Some(0));
    assert_eq!(st.editor.text(), "/", "buffer untouched by navigation");
}

// P4: Enter accepts the highlighted command and dispatches it.
#[test]
fn p4_enter_accepts_and_dispatches() {
    let mut st = TuiState::default();
    type_str(&mut st, "/ag");
    assert_eq!(
        tui::handle_key(&mut st, enter()),
        KeyAction::ToggleAgents,
        "accepting the only match runs it"
    );
    assert_eq!(st.editor.text(), "", "editor cleared after acceptance");
}

// P5: Tab completes the highlighted command into the buffer without
// submitting.
#[test]
fn p5_tab_completes_without_submit() {
    let mut st = TuiState::default();
    type_str(&mut st, "/ag");
    let r = tui::handle_key(&mut st, KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE));
    assert_eq!(r, KeyAction::Continue, "tab never submits");
    assert_eq!(st.editor.text(), "/agents", "completion lands in the buffer");
}

// P6: Esc closes the palette; the buffer stays as typed.
#[test]
fn p6_esc_closes_palette() {
    let mut st = TuiState::default();
    type_str(&mut st, "/ag");
    tui::handle_key(&mut st, KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
    assert!(st.palette_matches().is_none(), "palette closed");
    assert_eq!(st.editor.text(), "/ag", "buffer preserved");
}

// P7: an unknown command draws immediate feedback and NEVER becomes a
// mission (pre-fix: "/xyz" submitted as a goal and burned a mission).
#[test]
fn p7_unknown_command_feedback_not_mission() {
    let mut st = TuiState::default();
    type_str(&mut st, "/xyz");
    let r = tui::handle_key(&mut st, enter());
    assert_eq!(r, KeyAction::Continue, "no mission submit: {r:?}");
    let last = st.transcript.last().map(|l| l.to_string()).unwrap_or_default();
    assert!(
        last.contains("unknown command") && last.contains("/xyz"),
        "feedback names what was typed: {last}"
    );
    assert_eq!(st.editor.text(), "", "consumed input clears");
}

// P8: the colon alias drives the same registry.
#[test]
fn p8_colon_alias_parity() {
    let mut st = TuiState::default();
    type_str(&mut st, ":agents");
    assert_eq!(tui::handle_key(&mut st, enter()), KeyAction::ToggleAgents);
    let mut st = TuiState::default();
    type_str(&mut st, ":xyz");
    let r = tui::handle_key(&mut st, enter());
    assert_eq!(r, KeyAction::Continue);
    let last = st.transcript.last().map(|l| l.to_string()).unwrap_or_default();
    assert!(last.contains(":xyz"), "alias feedback keeps the typed sigil: {last}");
}

// P9: ordinary mission input never sees the palette.
#[test]
fn p9_plain_text_passthrough() {
    let mut st = TuiState::default();
    type_str(&mut st, "hello world");
    assert!(st.palette_matches().is_none(), "no palette for plain text");
    match tui::handle_key(&mut st, enter()) {
        KeyAction::Submit(t) => assert_eq!(t, "hello world"),
        other => panic!("expected Submit, got {other:?}"),
    }
}

// P10: bin-owned commands (status) pass through as submits; the palette
// does not swallow them.
#[test]
fn p10_bin_commands_passthrough() {
    let mut st = TuiState::default();
    type_str(&mut st, "/status");
    match tui::handle_key(&mut st, enter()) {
        KeyAction::Submit(t) => assert_eq!(t, "/status"),
        other => panic!("status reaches the bin: {other:?}"),
    }
}
