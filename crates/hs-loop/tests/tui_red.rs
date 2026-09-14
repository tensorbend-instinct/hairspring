//! UI gap #10 M1 (Eric 2026-09-08 14:52 via Main: full-screen TUI is THE
//! BUILD; loop substrate visible: phase indicator + event ticker +
//! delegation graph are first-class): ratatui-class full-screen surface.
//!
//! M1 contract: a four-region layout - transcript viewport (grows),
//! one-row loop rail, pinned composer box (3 rows), one-row HUD - that
//! renders into a ratatui `TestBackend` deterministically. Line mode stays
//! for piped stdin; this module is TTY-only at the bin seam.

use hs_loop::tui::{self, LoopPhase};
use ratatui::{Terminal, backend::TestBackend};

// R1: the layout splits the screen into the four regions, composer and
// HUD pinned at the bottom, viewport taking everything else.
#[test]
fn r1_four_region_layout() {
    let l = tui::layout(100, 30);
    assert_eq!(l.hud.y, 29, "HUD is the last row");
    assert_eq!(l.hud.height, 1);
    assert_eq!(l.composer.y, 26, "composer sits directly above the HUD");
    assert_eq!(
        l.composer.height, 3,
        "composer is a 3-row box (border, input, border)"
    );
    assert_eq!(l.rail.y, 25, "loop rail directly above the composer");
    assert_eq!(l.rail.height, 1);
    assert_eq!(l.viewport.y, 0);
    assert_eq!(
        l.viewport.height, 25,
        "viewport takes everything above the rail"
    );
    assert_eq!(l.viewport.width, 100);
}

// R2: the skeleton renders: composer box borders pinned at the bottom,
// HUD text on the last row.
#[test]
fn r2_skeleton_renders_pinned_composer_and_hud() {
    let backend = TestBackend::new(80, 24);
    let mut term = Terminal::new(backend).unwrap();
    term.draw(|f| tui::render_skeleton(f, &tui::TuiState::default()))
        .unwrap();
    let buf = term.backend().buffer();
    // Composer top border on row 20 (24 rows: viewport 0..19, rail 19?
    // layout: hud=23, composer=20..23, rail=19, viewport 0..19).
    let top_row: String = (0..80).map(|x| buf[(x, 20)].symbol()).collect();
    assert!(
        top_row.starts_with('\u{256d}'),
        "composer top-left corner: {top_row:?}"
    );
    assert!(
        top_row.ends_with('\u{256e}'),
        "composer top-right corner: {top_row:?}"
    );
    let bot_row: String = (0..80).map(|x| buf[(x, 22)].symbol()).collect();
    assert!(
        bot_row.starts_with('\u{2570}'),
        "composer bottom-left: {bot_row:?}"
    );
    assert!(
        bot_row.ends_with('\u{256f}'),
        "composer bottom-right: {bot_row:?}"
    );
    let hud: String = (0..80).map(|x| buf[(x, 23)].symbol()).collect();
    assert!(
        hud.contains("hs"),
        "footer carries the active model: {hud:?}"
    );
    assert!(
        !hud.contains("missions"),
        "normal footer omits machine counters: {hud:?}"
    );
    let input: String = (0..80).map(|x| buf[(x, 21)].symbol()).collect();
    assert!(
        input.contains("hs> "),
        "composer input line carries the prompt: {input:?}"
    );
}

// R3: the active status row names the current work without exposing the
// internal phase machine.
#[test]
fn r3_activity_row_is_task_focused() {
    let backend = TestBackend::new(80, 24);
    let mut term = Terminal::new(backend).unwrap();
    let state = tui::TuiState {
        phase: LoopPhase::Act,
        cur_step: 2,
        cur_action: "running tests".into(),
        ..Default::default()
    };
    term.draw(|f| tui::render_skeleton(f, &state)).unwrap();
    let buf = term.backend().buffer();
    let rail: String = (0..80).map(|x| buf[(x, 19)].symbol()).collect();
    assert!(
        rail.contains("step 2 · running tests"),
        "current work is visible: {rail:?}"
    );
    assert!(
        !rail.contains("PLAN") && !rail.contains("REFLECT"),
        "phase machine stays out of normal UI: {rail:?}"
    );
}

// R4: degenerate sizes never panic - below the minimum the surface
// degrades to just what fits.
#[test]
fn r4_tiny_terminal_never_panics() {
    for (w, h) in [(10, 3), (1, 1), (40, 4), (0, 0)] {
        let backend = TestBackend::new(w, h);
        let mut term = Terminal::new(backend).unwrap();
        term.draw(|f| tui::render_skeleton(f, &tui::TuiState::default()))
            .unwrap();
    }
}

// R5: stream events remain recorded in state for overlays and diagnostics;
// they do not become mystery glyphs on the normal status row.
#[test]
fn r5_event_ticker_is_not_rendered_as_mystery_glyphs() {
    let backend = TestBackend::new(80, 24);
    let mut term = Terminal::new(backend).unwrap();
    let mut state = tui::TuiState::default();
    state.push_stream_event(hs_core::EventKind::ModelCall);
    state.push_stream_event(hs_core::EventKind::ToolCall);
    state.push_stream_event(hs_core::EventKind::Observation);
    term.draw(|f| tui::render_skeleton(f, &state)).unwrap();
    let buf = term.backend().buffer();
    let rail: String = (0..80).map(|x| buf[(x, 19)].symbol()).collect();
    assert!(
        rail.trim().is_empty(),
        "idle event history is not UI chrome: {rail:?}"
    );
}
