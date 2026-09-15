//! UI gap #10 M3: the transcript viewport + markdown adapter. The
//! line-mode `MarkdownStreamer` emits ANSI strings; the full-screen
//! surface needs ratatui Lines styled from the Theme. The viewport
//! holds completed transcript lines, auto-follows the bottom, and
//! scrolls back on demand.

use hs_loop::tui::{self, TuiState};
use hs_loop::uipaint::Theme;
use ratatui::{Terminal, backend::TestBackend, style::Modifier};

// R1: the markdown adapter converts a stream line into styled ratatui
// Lines: headers bold+underlined, inline code accented, bullets dotted,
// fences verbatim + dim.
#[test]
fn r1_markdown_to_styled_lines() {
    let theme = Theme::dark();
    let lines = tui::md_to_lines("## Result\nUse `cargo test` now.\n- one\n- two", &theme);
    assert_eq!(lines.len(), 4);
    let header: String = lines[0].spans.iter().map(|s| s.content.clone()).collect();
    assert_eq!(header, "Result", "header marker stripped");
    assert!(
        lines[0]
            .spans
            .iter()
            .all(|s| s.style.add_modifier.contains(Modifier::BOLD))
    );
    let code_span = lines[1]
        .spans
        .iter()
        .find(|s| s.content.contains("cargo test"))
        .expect("code span present");
    assert_eq!(code_span.style.fg, Some(ratatui::style::Color::Cyan));
    let bullet: String = lines[2].spans.iter().map(|s| s.content.clone()).collect();
    assert!(bullet.contains('\u{2022}'), "bullet glyph: {bullet:?}");
}

// R2: fence blocks pass through verbatim (no inline parsing), dimmed.
#[test]
fn r2_fence_verbatim() {
    let theme = Theme::dark();
    let lines = tui::md_to_lines("```\n**not bold** `not code`\n```", &theme);
    assert_eq!(lines.len(), 1, "fence markers consumed, one body line kept");
    let body: String = lines[0].spans.iter().map(|s| s.content.clone()).collect();
    assert_eq!(body, "**not bold** `not code`");
}

// R3: viewport auto-follows: with more lines than rows, the tail shows.
#[test]
fn r3_viewport_follows_tail() {
    let backend = TestBackend::new(40, 24);
    let mut term = Terminal::new(backend).unwrap();
    let mut st = TuiState::default();
    for i in 0..30 {
        st.push_transcript_line(&format!("line {i:02}"));
    }
    term.draw(|f| tui::render_skeleton(f, &st)).unwrap();
    let buf = term.backend().buffer();
    let top: String = (0..40).map(|x| buf[(x, 0)].symbol()).collect();
    let bottom: String = (0..40).map(|x| buf[(x, 16)].symbol()).collect();
    assert!(
        bottom.contains("line 29"),
        "newest line at the viewport bottom: {bottom:?}"
    );
    assert!(
        top.contains("HAIRSPRING: Session"),
        "slim session header stays above the tail: {top:?}"
    );
    assert!(!top.contains("line 00"), "oldest lines scrolled off");
}

// R4: scroll back pins the view; new lines arrive but the window holds;
// scrolling to the bottom re-engages follow.
#[test]
fn r4_scrollback_pins_and_refollows() {
    let mut st = TuiState::default();
    for i in 0..30 {
        st.push_transcript_line(&format!("line {i:02}"));
    }
    st.transcript_scroll_up(5);
    let backend = TestBackend::new(40, 24);
    let mut term = Terminal::new(backend).unwrap();
    term.draw(|f| tui::render_skeleton(f, &st)).unwrap();
    let bottom: String = (0..40)
        .map(|x| term.backend().buffer()[(x, 16)].symbol())
        .collect();
    assert!(
        bottom.contains("line 24"),
        "scrolled 5 up from the tail: {bottom:?}"
    );
    // New activity does not yank a scrolled-back view.
    st.push_transcript_line("line 30");
    term.draw(|f| tui::render_skeleton(f, &st)).unwrap();
    let bottom2: String = (0..40)
        .map(|x| term.backend().buffer()[(x, 16)].symbol())
        .collect();
    assert!(
        bottom2.contains("line 24"),
        "pinned while scrolled: {bottom2:?}"
    );
    st.transcript_scroll_to_bottom();
    term.draw(|f| tui::render_skeleton(f, &st)).unwrap();
    let bottom3: String = (0..40)
        .map(|x| term.backend().buffer()[(x, 16)].symbol())
        .collect();
    assert!(
        bottom3.contains("line 30"),
        "follow re-engaged: {bottom3:?}"
    );
}

// R5: markdown conversion feeds the viewport: a mission answer's
/// markdown lands as styled transcript lines.
#[test]
fn r5_markdown_into_viewport() {
    let backend = TestBackend::new(60, 24);
    let mut term = Terminal::new(backend).unwrap();
    let mut st = TuiState::default();
    st.push_transcript_markdown("## Answer\nDone with `zero` defects.", &Theme::dark());
    term.draw(|f| tui::render_skeleton(f, &st)).unwrap();
    let buf = term.backend().buffer();
    let row0: String = (0..60).map(|x| buf[(x, 1)].symbol()).collect();
    let row1: String = (0..60).map(|x| buf[(x, 2)].symbol()).collect();
    assert!(row0.contains("Answer"), "header rendered: {row0:?}");
    assert!(row1.contains("zero"), "inline code rendered: {row1:?}");
}
