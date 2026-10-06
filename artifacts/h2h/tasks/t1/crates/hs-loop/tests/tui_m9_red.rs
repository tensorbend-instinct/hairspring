//! UI gap #10 M9 RED: the full-screen surface honors `HS_THEME`.
//!
//! Hostile-review finding: `TuiState` hard-coded `Theme::dark()` for
//! markdown flushes and `Color::Cyan` for chrome accents, so
//! `HS_THEME=light` changed nothing on the TUI surface while line mode
//! switched correctly. The state carries the theme; every style on
//! the surface derives from it.

use hs_loop::tui::{self, LoopPhase, TuiState};
use hs_loop::uipaint::{Theme, UiEvent};
use ratatui::{backend::TestBackend, Terminal};

fn flush_code_span(st: &mut TuiState) -> ratatui::style::Style {
    st.on_answer_delta("`x`");
    st.on_ui_event(&UiEvent::ModelCallEnd {
        model: "m".into(),
        input_tokens: 1,
        output_tokens: 1,
        cost_usd_micros: 100,
    });
    // M19: the call boundary no longer commits; the disposition
    // boundary does (mission end here).
    st.commit_answer_tail();
    st.transcript
        .last()
        .and_then(|l| l.spans.first())
        .map(|s| s.style)
        .expect("flushed code span")
}

// R1: default state keeps the dark theme (regression guard).
#[test]
fn r1_default_theme_is_dark() {
    let mut st = TuiState::default();
    assert_eq!(st.theme, Theme::dark(), "default surface theme is dark");
    let style = flush_code_span(&mut st);
    assert_eq!(
        style.fg,
        Some(ratatui::style::Color::Cyan),
        "dark theme renders inline code cyan"
    );
}

// R2: a light-theme state flushes markdown with the LIGHT roles.
#[test]
fn r2_light_theme_drives_markdown_flush() {
    let mut st = TuiState {
        theme: Theme::light(),
        ..Default::default()
    };
    let style = flush_code_span(&mut st);
    assert_eq!(
        style.fg,
        Some(ratatui::style::Color::Blue),
        "light theme renders inline code blue, got {style:?}"
    );
}

// R3: the active task header accent follows the state theme; the phase rail is no longer normal chrome.
#[test]
fn r3_task_header_accent_follows_theme() {
    for (theme,want) in [(Theme::dark(),ratatui::style::Color::Cyan),(Theme::light(),ratatui::style::Color::Blue)] {
        let state=TuiState{phase:LoopPhase::Act,session_title:"Work".into(),theme,..Default::default()};
        let mut term=Terminal::new(TestBackend::new(80,24)).unwrap(); term.draw(|f|tui::render_skeleton(f,&state)).unwrap(); let buf=term.backend().buffer();
        let row:String=(0..80).map(|x|buf[(x,0)].symbol()).collect(); let pos=row.find("HAIRSPRING: Work").unwrap() as u16;
        assert_eq!(buf[(pos,0)].style().fg,Some(want),"task header uses state theme accent");
        assert!(!row.contains("ACT"),"internal phase is not normal chrome");
    }
}
