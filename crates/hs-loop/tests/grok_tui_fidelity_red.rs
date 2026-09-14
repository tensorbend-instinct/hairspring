//! RED-first visual hierarchy gate for the grok-cli/OpenTUI design.
//! Source: superagent-ai/grok-cli src/ui/app.tsx and src/ui/theme.ts.

use hs_loop::tui::{self, TuiState};
use ratatui::{Terminal, backend::TestBackend};

fn screen(st: &TuiState, w: u16, h: u16) -> String {
    let mut terminal = Terminal::new(TestBackend::new(w, h)).unwrap();
    terminal.draw(|f| tui::render_skeleton(f, st)).unwrap();
    let b = terminal.backend().buffer();
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

#[test]
fn empty_home_centers_identity_and_prompt_instead_of_showing_an_empty_dashboard() {
    let mut st = TuiState::default();
    st.model_label = "DeepSeek V4 Pro".into();
    st.cwd_label = "~/hairspring".into();
    let s = screen(&st, 80, 24);
    let _ = std::fs::write("/tmp/hairspring-grok-home.txt", &s);
    assert!(s.contains("HAIRSPRING"), "home identity: {s}");
    assert!(
        s.contains("What are we building?"),
        "single central prompt: {s}"
    );
    assert!(s.contains("DeepSeek V4 Pro"), "model next to prompt: {s}");
    assert!(s.contains("~/hairspring"), "quiet cwd footer: {s}");
    assert!(
        !s.contains("OBSERVE › ACT › CHECK › REFLECT"),
        "idle machinery must not dominate home: {s}"
    );
}

#[test]
fn active_session_has_one_header_message_canvas_prompt_and_contextual_footer() {
    let mut st = TuiState::default();
    st.session_title = "Fix the verifier".into();
    st.stream_short = "abc123".into();
    st.model_label = "DeepSeek V4 Pro".into();
    st.cwd_label = "~/hairspring".into();
    st.context_remaining_pct = Some(73);
    st.cur_step = 1;
    st.cur_action = "thinking".into();
    st.push_goal_echo("Find and fix the bug");
    let s = screen(&st, 100, 28);
    assert!(
        s.contains("HAIRSPRING: Fix the verifier"),
        "clear session header: {s}"
    );
    assert!(s.contains("abc123"), "session id at header edge: {s}");
    assert!(s.contains("Find and fix the bug"), "message canvas: {s}");
    assert!(
        s.contains("Queue a follow-up"),
        "prompt says what Enter does while active: {s}"
    );
    assert!(s.contains("73% context"), "model/context meter: {s}");
    assert!(
        s.contains("enter queue") && s.contains("esc interrupt"),
        "state-aware shortcuts: {s}"
    );
    assert!(s.contains("~/hairspring"), "cwd footer: {s}");
}

#[test]
fn tool_activity_stays_inline_and_never_opens_a_permanent_machine_dashboard() {
    use hs_loop::uipaint::UiEvent;
    let mut st = TuiState::default();
    st.push_goal_echo("inspect files");
    st.on_ui_event(&UiEvent::ToolCallStart {
        plugin: "repo.search".into(),
        args_summary: "verifier".into(),
    });
    let s = screen(&st, 90, 26);
    assert!(
        s.contains("▣") && s.contains("repo.search"),
        "inline tool row: {s}"
    );
    assert!(
        !s.contains("T_mission") && !s.contains("lineage") && !s.contains("scorer"),
        "advanced machinery remains on-demand: {s}"
    );
}
