//! Beat 5 RED (Eric 2026-09-10: hostile visual/product review vs
//! Codex CLI / Claude Code / exo; fix the three weakest points found
//! in rendered pixels):
//!
//! W3 v1: Esc closing the palette left the sigil in the composer -
//! the next plain-text goal became "/write hello.txt ..." and the
//! surface answered "unknown command" (live capture cap-22, 60x20).
//! W1 v2: the mission outcome - the product's key moment - rendered
//! as a dim undifferentiated line; pass and fail looked identical
//! (cap-06).
//! W2 v3: overlay panel content CLIPS at the box border (ratatui
//! Paragraph truncates); the T_mission report's long rows were cut
//! mid-word (cap-11). A clipped number is a wrong number.

use hs_loop::mission_time::Decomposition;
use hs_loop::tui::{self, KeyAction, TuiState};
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::style::Modifier;

fn key(c: char) -> KeyEvent {
    KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE)
}

// v1: Esc with the palette open closes it AND clears the sigil
// buffer, so ordinary mission input can never be silently re-read as
// a command.
#[test]
fn v1_esc_clears_sigil_buffer() {
    let mut st = TuiState::default();
    for c in "/ag".chars() {
        tui::handle_key(&mut st, key(c));
    }
    assert!(st.palette.is_some(), "palette open on /ag");
    tui::handle_key(&mut st, KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
    assert!(st.palette.is_none(), "Esc closes the palette");
    assert_eq!(
        st.editor.text(),
        "",
        "Esc must not leave a stray sigil in the composer"
    );
    // The exact live-capture regression: type a plain goal next.
    for c in "write hello.txt".chars() {
        tui::handle_key(&mut st, key(c));
    }
    let a = tui::handle_key(&mut st, KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    assert_eq!(a, KeyAction::Submit("write hello.txt".to_string()));
    assert!(
        st.transcript.is_empty(),
        "no unknown-command feedback for a plain goal"
    );
}

// v1b: Esc with the palette closed leaves a plain-text buffer alone.
#[test]
fn v1b_esc_keeps_plain_text() {
    let mut st = TuiState::default();
    for c in "draft goal".chars() {
        tui::handle_key(&mut st, key(c));
    }
    tui::handle_key(&mut st, KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
    assert_eq!(st.editor.text(), "draft goal");
}

// v2: pass and fail outcomes read differently at a glance; both are
// bold. Text content is unchanged (M14/M22 format parity).
#[test]
fn v2_done_line_verdict_styling() {
    let mut ok = TuiState::default();
    ok.mission_done_report(9, 10, 2100, 2100, "verified");
    let pass = ok.transcript.last().unwrap().clone();
    assert!(
        pass.spans.iter().any(|s| s.style.add_modifier.contains(Modifier::BOLD)),
        "outcome line bold: {pass:?}"
    );
    let mut bad = TuiState::default();
    bad.mission_done_report(50, 50, 35000, 35000, "steps_exhausted");
    let fail = bad.transcript.last().unwrap().clone();
    assert!(
        fail.spans.iter().any(|s| s.style.add_modifier.contains(Modifier::BOLD)),
        "outcome line bold: {fail:?}"
    );
    let pass_fg = pass.spans[0].style.fg;
    let fail_fg = fail.spans[0].style.fg;
    assert_ne!(
        pass_fg, fail_fg,
        "pass and fail outcomes must be distinguishable at a glance"
    );
    // format parity: the line text still matches the backfill format
    let text: String = pass.spans.iter().map(|s| s.content.as_ref()).collect();
    assert!(text.contains("done: 9 steps, 10 calls"), "M14 format: {text}");
    assert!(text.contains("(verified)"), "M22 outcome: {text}");
}

// v3: panel content wraps to the box's inner width; nothing clips at
// the border. Mechanized at the helper the render path uses.
#[test]
fn v3_panel_lines_wrap_within_box() {
    let long = "S stuck repeats=78 ms=2991 (normalized-signature duplicate tool calls beyond the recovery budget; the loop kept re-issuing the same call)";
    let lines = vec![ratatui::text::Line::from(long.to_string())];
    let wrapped = tui::wrap_panel_lines(&lines, 40);
    assert!(wrapped.len() > 1, "long line wraps: {wrapped:?}");
    for l in &wrapped {
        assert!(l.width() <= 40, "every row fits the box: {l:?}");
    }
    let joined: String = wrapped
        .iter()
        .flat_map(|l| l.spans.iter().map(|s| s.content.to_string()))
        .collect::<Vec<_>>()
        .join("");
    let collapsed: String = joined.split_whitespace().collect();
    let original: String = long.split_whitespace().collect();
    assert_eq!(collapsed, original, "no content lost to wrapping");
}

// v3b: the T_mission panel's real report lines fit the box at 100x30.
#[test]
fn v3b_time_panel_content_fits() {
    let d = Decomposition {
        n_steps: 100,
        t_model_ms: 258,
        t_overhead_ms: 2781,
        r_failures: 0,
        t_recover_ms: 0,
        c_coord_events: 0,
        c_coord_ms: 0,
        s_stuck_repeats: 78,
        s_stuck_ms: 2991,
        wall_ms: 12849,
        unattributed_ms: 6819,
    };
    let inner = tui::panel_inner_width(100);
    let lines = tui::time_panel_lines(&d, inner);
    assert!(!lines.is_empty());
    for l in &lines {
        assert!(
            l.width() <= inner as usize,
            "report row fits the panel: {:?} ({} > {inner})",
            l,
            l.width()
        );
    }
}
