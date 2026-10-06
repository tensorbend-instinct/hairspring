//! Live grok comparison supersedes the permanent loop-rail contract.
use hs_loop::tui::{self, TuiState};
use ratatui::{Terminal, backend::TestBackend};
fn screen(st: &TuiState, w: u16, h: u16) -> String {
    let mut t = Terminal::new(TestBackend::new(w, h)).unwrap();
    t.draw(|f| tui::render_skeleton(f, st)).unwrap();
    let b = t.backend().buffer();
    (0..h)
        .map(|y| (0..w).map(|x| b[(x, y)].symbol()).collect::<String>())
        .collect::<Vec<_>>()
        .join("\n")
}
#[test]
fn normal_surface_hides_internal_phase_rail_at_40_cols() {
    let mut s = TuiState::default();
    s.push_stream_event(hs_core::EventKind::ModelCall);
    let x = screen(&s, 40, 12);
    assert!(!x.contains("PLAN") && !x.contains("REFLECT"));
    assert!(x.contains("Agent"));
}
#[test]
fn narrow_surface_keeps_prompt_not_ticker() {
    let mut s = TuiState::default();
    s.push_stream_event(hs_core::EventKind::ToolCall);
    let x = screen(&s, 24, 12);
    assert!(!x.contains('◆') && !x.contains('⚙'));
    assert!(x.contains("Agent"));
}
