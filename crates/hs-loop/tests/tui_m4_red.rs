//! UI gap #10 M4: pickers as overlays, mouse scroll, resize.
//!
//! Contract: the resume picker (line-mode gap #7) becomes an overlay
//! list on the full-screen surface; PgUp/PgDn and the mouse wheel drive
//! transcript scrollback; a resize reflows the layout without losing
//! state.

use hs_loop::tui::{self, TuiState};
use ratatui::{backend::TestBackend, Terminal};

// R1: the picker overlay renders centered over the transcript with a
// numbered, highlighted selection.
#[test]
fn r1_picker_overlay_renders() {
    let backend = TestBackend::new(80, 24);
    let mut term = Terminal::new(backend).unwrap();
    let mut st = TuiState::default();
    st.open_picker(vec![
        "1  ca8c2df6  scripted  2026-09-08 14:20  probe the composer".to_string(),
        "2  20c0dff2  scripted  2026-09-08 13:05  status bar probe".to_string(),
    ]);
    term.draw(|f| tui::render_skeleton(f, &st)).unwrap();
    let buf = term.backend().buffer();
    let mut found = (false, false);
    for y in 0..24 {
        let row: String = (0..80).map(|x| buf[(x, y)].symbol()).collect();
        if row.contains("ca8c2df6") {
            found.0 = true;
        }
        if row.contains("20c0dff2") {
            found.1 = true;
        }
    }
    assert!(found.0 && found.1, "both sessions visible in the overlay");
    // Selection highlight: row of entry 1 carries the accent color.
    let mut sel_row = None;
    for y in 0..24 {
        let row: String = (0..80).map(|x| buf[(x, y)].symbol()).collect();
        if row.contains("ca8c2df6") {
            sel_row = Some(y);
        }
    }
    let y = sel_row.unwrap();
    let accented = (0..80u16).any(|x| {
        buf[(x, y)].style().fg == Some(ratatui::style::Color::Cyan)
            || buf[(x, y)].style().bg == Some(ratatui::style::Color::Cyan)
            || buf[(x, y)].style().bg == Some(ratatui::style::Color::DarkGray)
    });
    assert!(accented, "selected entry is highlighted");
}

// R2: picker navigation moves the selection; enter takes the choice;
// esc cancels back to the transcript.
#[test]
fn r2_picker_navigation() {
    let mut st = TuiState::default();
    st.open_picker(vec!["a".to_string(), "b".to_string(), "c".to_string()]);
    assert_eq!(st.picker_selected(), Some(0));
    st.picker_down();
    assert_eq!(st.picker_selected(), Some(1));
    st.picker_down();
    st.picker_down();
    assert_eq!(st.picker_selected(), Some(2), "clamped at the last entry");
    st.picker_up();
    assert_eq!(st.picker_selected(), Some(1));
    assert_eq!(
        st.picker_take(),
        Some((hs_loop::tui::PickerKind::Resume, "b".to_string()))
    );
    assert_eq!(st.picker_selected(), None, "closed after take");
    st.open_picker(vec!["x".to_string()]);
    st.picker_cancel();
    assert_eq!(st.picker_selected(), None);
    assert_eq!(st.picker_take(), None);
}

// R3: PgUp/PgDn scroll the transcript by a page; the wheel scrolls by
// 3 lines; scrolling past the top clamps.
#[test]
fn r3_page_and_wheel_scroll() {
    let mut st = TuiState::default();
    for i in 0..60 {
        st.push_transcript_line(&format!("line {i:02}"));
    }
    st.transcript_page_up(19); // one viewport page
    let backend = TestBackend::new(40, 24);
    let mut term = Terminal::new(backend).unwrap();
    term.draw(|f| tui::render_skeleton(f, &st)).unwrap();
    let bottom: String = (0..40).map(|x| term.backend().buffer()[(x, 18)].symbol()).collect();
    assert!(bottom.contains("line 40"), "paged up 19 from 59: {bottom:?}");
    st.transcript_wheel_up(3);
    term.draw(|f| tui::render_skeleton(f, &st)).unwrap();
    let bottom2: String = (0..40).map(|x| term.backend().buffer()[(x, 18)].symbol()).collect();
    assert!(bottom2.contains("line 37"), "wheel up 3 more: {bottom2:?}");
    st.transcript_page_up(19);
    st.transcript_page_up(19);
    st.transcript_page_up(19);
    term.draw(|f| tui::render_skeleton(f, &st)).unwrap();
    let top: String = (0..40).map(|x| term.backend().buffer()[(x, 0)].symbol()).collect();
    assert!(top.contains("line 00"), "clamped at the very top: {top:?}");
    st.transcript_wheel_down(3);
    term.draw(|f| tui::render_skeleton(f, &st)).unwrap();
    // Window is lines[0..4]: row 0 holds line 00, row 3 holds line 03.
    let row3: String = (0..40).map(|x| term.backend().buffer()[(x, 3)].symbol()).collect();
    assert!(row3.contains("line 03"), "wheel back down 3: {row3:?}");
}

// R4: resize keeps state and re-lays-out: same transcript, new rects.
#[test]
fn r4_resize_reflows() {
    let mut st = TuiState::default();
    for i in 0..10 {
        st.push_transcript_line(&format!("row {i}"));
    }
    let small = TestBackend::new(60, 15);
    let mut term = Terminal::new(small).unwrap();
    term.draw(|f| tui::render_skeleton(f, &st)).unwrap();
    let hud_small: String = (0..60).map(|x| term.backend().buffer()[(x, 14)].symbol()).collect();
    assert!(hud_small.contains("missions"), "HUD on the last row at 15 rows");
    let big = TestBackend::new(120, 40);
    let mut term2 = Terminal::new(big).unwrap();
    term2.draw(|f| tui::render_skeleton(f, &st)).unwrap();
    let hud_big: String = (0..120).map(|x| term2.backend().buffer()[(x, 39)].symbol()).collect();
    assert!(hud_big.contains("missions"), "HUD on the last row at 40 rows");
    assert_eq!(st.transcript.len(), 10, "transcript untouched by resize");
}
