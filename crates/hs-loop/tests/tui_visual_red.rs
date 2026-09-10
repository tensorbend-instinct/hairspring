//! RED: the TUI transcript and chrome must speak the SAME color
//! language the line-mode painter (uipaint) and the reference CLIs
//! (Claude Code, Grok Build) use: themed arrows/tool names, verdict
//! colors on results, dim chrome borders with accented labels, a
//! colored HUD. Today the TUI renders nearly monochrome while the
//! theme roles sit unused (live capture cap-06 audit, 2026-09-10).

use hs_loop::tui::{self, TuiState};
use hs_loop::uipaint::{Theme, UiEvent};
use ratatui::backend::TestBackend;
use ratatui::style::{Color, Modifier};
use ratatui::Terminal;

fn render(st: &TuiState, w: u16, h: u16) -> ratatui::buffer::Buffer {
    let backend = TestBackend::new(w, h);
    let mut term = Terminal::new(backend).unwrap();
    term.draw(|f| tui::render_skeleton(f, st)).unwrap();
    term.backend().buffer().clone()
}

/// Find all cells whose symbol equals `sym`; returns (x, y, fg, modifier).
fn cells(buf: &ratatui::buffer::Buffer, sym: &str, w: u16, h: u16) -> Vec<(u16, u16, Color, Modifier)> {
    let mut out = Vec::new();
    for y in 0..h {
        for x in 0..w {
            let c = &buf[(x, y)];
            if c.symbol() == sym {
                out.push((x, y, c.fg, c.modifier));
            }
        }
    }
    out
}

fn row_text(buf: &ratatui::buffer::Buffer, y: u16, w: u16) -> String {
    (0..w).map(|x| buf[(x, y)].symbol()).collect()
}

#[test]
fn tool_call_lines_carry_theme_colors() {
    let mut st = TuiState::default();
    st.on_ui_event(&UiEvent::ToolCallStart {
        plugin: "term.exec".into(),
        args_summary: "ls -la".into(),
    });
    st.on_ui_event(&UiEvent::ToolCallEnd {
        plugin: "term.exec".into(),
        ok: true,
        output_summary: "done".into(),
        elapsed_ms: 42,
    });
    let buf = render(&st, 80, 24);
    // The arrow speaks the theme accent (line mode: accent)...
    let arrows = cells(&buf, "\u{25b6}", 80, 24);
    assert!(!arrows.is_empty(), "tool arrow rendered");
    let (_, _, fg, _) = arrows[0];
    assert_eq!(fg, Color::Cyan, "tool arrow in theme accent (36;1): {fg:?}");
    // ...the plugin name speaks the tool color...
    let trow = arrows[0].1;
    let text = row_text(&buf, trow, 80);
    assert!(text.contains("term.exec"), "plugin on the arrow row: {text}");
    let name_x = text.find("term.exec").unwrap() as u16;
    assert_eq!(buf[(name_x, trow)].fg, Color::Cyan, "plugin in theme tool color");
    // ...and the ok verdict speaks theme.ok (green).
    let checks = cells(&buf, "\u{2713}", 80, 24);
    assert!(!checks.is_empty(), "ok check rendered");
    assert_eq!(checks[0].2, Color::Green, "ok verdict in theme.ok (32)");
}

#[test]
fn failed_tool_call_speaks_fail_color() {
    let mut st = TuiState::default();
    st.on_ui_event(&UiEvent::ToolCallEnd {
        plugin: "term.exec".into(),
        ok: false,
        output_summary: "boom".into(),
        elapsed_ms: 7,
    });
    let buf = render(&st, 80, 24);
    let marks = cells(&buf, "\u{2717}", 80, 24);
    assert!(!marks.is_empty(), "fail mark rendered");
    assert_eq!(marks[0].2, Color::Red, "fail verdict in theme.fail (31;1)");
}

#[test]
fn goal_echo_arrow_is_accented() {
    let mut st = TuiState::default();
    st.push_goal_echo("alpha");
    let buf = render(&st, 80, 24);
    // row 0 is the transcript top; the rail lives near the bottom, so
    // scope to the viewport rows (y < 19 at h=24).
    let arrows: Vec<_> = cells(&buf, "\u{203a}", 80, 24)
        .into_iter()
        .filter(|&(_, y, _, _)| y < 19)
        .collect();
    assert!(!arrows.is_empty(), "goal echo arrow rendered");
    assert_eq!(arrows[0].2, Color::Cyan, "goal echo arrow in theme accent");
}

#[test]
fn composer_border_is_dim_and_title_accented() {
    let mut st = TuiState::default();
    st.model_label = "scripted".into();
    let buf = render(&st, 80, 24);
    // Top-left composer corner is the ╭ glyph on row h-5+1 = 20 at h=24.
    let corners = cells(&buf, "\u{256d}", 80, 24);
    assert!(!corners.is_empty(), "composer corner rendered");
    let (_, _, _, m) = corners[0];
    assert!(m.contains(Modifier::DIM), "composer border dim: {m:?}");
    // The model label on the border row speaks the accent.
    let (_, cy, _, _) = corners[0];
    let text = row_text(&buf, cy, 80);
    assert!(text.contains("scripted"), "composer title on border row: {text}");
    let lx = text.find("scripted").unwrap() as u16;
    assert_eq!(buf[(lx, cy)].fg, Color::Cyan, "composer model label accented");
}

#[test]
fn hud_separators_dim_and_cost_colored() {
    let mut st = TuiState::default();
    st.missions_run = 1;
    st.total_steps = 2;
    st.total_model_calls = 3;
    st.total_cost_micros = 2100;
    st.stream_short = "abc123".into();
    let buf = render(&st, 80, 24);
    let hud = row_text(&buf, 23, 80);
    assert!(hud.contains("$0.0021"), "hud cost rendered: {hud}");
    let cx = hud.find("$0.0021").unwrap() as u16;
    assert_eq!(buf[(cx, 23)].fg, Color::Yellow, "hud cost in theme.cost (33)");
    let sx = hud.find("\u{00b7}").unwrap() as u16;
    assert!(buf[(sx, 23)].modifier.contains(Modifier::DIM), "hud separators dim");
}

#[test]
fn markdown_h1_is_a_header_and_levels_differ() {
    let theme = Theme::dark();
    let lines = tui::md_to_lines("# Big\n## Mid\nplain", &theme);
    assert_eq!(lines.len(), 3);
    let h1 = &lines[0].spans[0];
    assert!(
        h1.style.fg.is_some() || h1.style.add_modifier.contains(Modifier::BOLD),
        "h1 styled as a header: {:?}",
        h1.style
    );
    let h2 = &lines[1].spans[0];
    assert!(h2.style.add_modifier.contains(Modifier::BOLD), "h2 bold");
    assert!(
        !h2.style.add_modifier.contains(Modifier::UNDERLINED),
        "h2 must not carry the h1 underline"
    );
}
