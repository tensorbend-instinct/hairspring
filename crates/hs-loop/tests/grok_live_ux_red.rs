//! RED-first contracts from side-by-side PTY runs of grok-cli fb97af83 and HAIRSPRING.
use hs_loop::tui::{self, KeyAction, TuiState};
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::{Terminal, backend::TestBackend};
fn screen(st: &TuiState, w: u16, h: u16) -> String {
    let mut t = Terminal::new(TestBackend::new(w, h)).unwrap();
    t.draw(|f| tui::render_skeleton(f, st)).unwrap();
    let b = t.backend().buffer();
    (0..h)
        .map(|y| {
            (0..w)
                .map(|x| b[(x, y)].symbol())
                .collect::<String>()
                .trim_end()
                .to_string()
        })
        .collect::<Vec<_>>()
        .join("\n")
}
fn type_text(st: &mut TuiState, s: &str) {
    for c in s.chars() {
        assert_eq!(
            tui::handle_key(st, KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE)),
            KeyAction::Continue
        );
    }
}
#[test]
fn slash_help_opens_searchable_overlay_instead_of_appending_a_wall_of_text() {
    let mut st = TuiState::default();
    type_text(&mut st, "/help");
    let action = tui::handle_key(&mut st, KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    assert_eq!(
        action,
        KeyAction::Continue,
        "/help is a UI transition, not bin transcript output"
    );
    let s = screen(&st, 100, 32);
    assert!(
        s.contains("Commands") && s.contains("Search..."),
        "grok live /help modal hierarchy: {s}"
    );
    assert!(
        st.help_overlay,
        "help must remain keyboard navigable and escape-dismissable"
    );
}
#[test]
fn escape_interrupts_an_active_run_like_the_live_reference() {
    let mut st = TuiState::default();
    st.phase = tui::LoopPhase::Act;
    assert_eq!(
        tui::handle_key(&mut st, KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)),
        KeyAction::Interrupt
    );
}

#[test]
fn prompt_is_quiet_borderless_row_not_a_large_box() {
    let mut st = TuiState::default();
    st.model_label = "scripted".into();
    let s = screen(&st, 100, 32);
    assert!(
        s.contains("Agent  What are we building?"),
        "live reference labels the one quiet prompt row: {s}"
    );
    assert!(
        !s.contains('╭') && !s.contains('╰'),
        "prompt must not be a bordered dashboard widget: {s}"
    );
}
#[test]
fn typing_slash_opens_centered_searchable_command_discovery() {
    let mut st = TuiState::default();
    type_text(&mut st, "/");
    let s = screen(&st, 60, 20);
    assert!(
        s.contains("Commands") && s.contains("Search..."),
        "live slash overlay: {s}"
    );
    assert!(
        s.find("Commands").unwrap_or(usize::MAX)
            < s.find("What are we building?").unwrap_or(usize::MAX),
        "overlay occupies canvas rather than attaching to composer: {s}"
    );
}

#[test]
fn narrow_home_does_not_clip_an_instruction_banner() {
    let st = TuiState::default();
    let s = screen(&st, 60, 20);
    assert!(
        !s.contains("type a goal and press Enter"),
        "live narrow home relies on prompt, without clipped duplicate instructions: {s}"
    );
}
#[test]
fn model_picker_has_the_same_search_first_hierarchy() {
    let mut st = TuiState::default();
    st.open_picker_kind(
        tui::PickerKind::Models,
        vec!["scripted (current)".into(), "+ Add provider...".into()],
    );
    let s = screen(&st, 100, 32);
    assert!(
        s.contains("Select model")
            && s.contains("Search...")
            && s.contains("enter select")
            && s.contains("esc close"),
        "live picker hierarchy: {s}"
    );
}
