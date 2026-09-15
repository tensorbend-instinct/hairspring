//! RED-first contracts measured from real Exo e1548a2 and Pi v0.37.0 PTY captures.
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
fn exo_runtime_hierarchy_has_distinct_transcript_and_titled_input_boundaries() {
    let mut st = TuiState::default();
    st.session_title = "capture".into();
    st.push_goal_echo("hello tool hierarchy");
    let s = screen(&st, 100, 32);
    assert!(
        s.contains("┌ HAIRSPRING: capture"),
        "measured Exo outer transcript boundary: {s}"
    );
    assert!(
        s.contains("┌ message or /command"),
        "measured Exo distinct input boundary: {s}"
    );
    assert!(
        s.contains("└") && s.contains("│"),
        "boundaries must be visually closed: {s}"
    );
}

#[test]
fn pi_reasoning_is_not_wrapped_in_an_extra_heavy_box() {
    let mut st = TuiState::default();
    st.push_goal_echo("inspect");
    st.answer_inflight = "Reasoning through the change".into();
    let s = screen(&st, 100, 32);
    assert!(
        !s.contains("╭ Reasoning") && !s.contains("┌ Reasoning"),
        "Pi v0.37.0 renders muted reasoning and a separator, not a per-reasoning box: {s}"
    );
}
