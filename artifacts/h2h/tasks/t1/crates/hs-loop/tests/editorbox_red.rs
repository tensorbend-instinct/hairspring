//! REPL UI gap #8 (Eric 2026-09-08: "just fix the gaps" + 13:12
//! look-and-feel steering): pi/omp frame the input in a bordered
//! composer box; hs-repl shows a bare "hs> " with no visual structure.
//!
//! Contract: a line-based composer frame for the interactive prompt -
//! a top border carrying the session label, a left-bar prompt, a bottom
//! border - painted with box-drawing glyphs, dim chrome + bright label
//! on a terminal, ANSI-free when piped. Every frame line's VISIBLE
//! width (escape codes excluded) equals the requested column count.

use hs_loop::uipaint::{composer_bottom, composer_top, separator, visible_width, EDITOR_PROMPT};

// R1: the top border frames the label, box glyphs at both ends,
// exact visible width, ANSI on a terminal.
#[test]
fn r1_composer_top_frames_label() {
    let s = composer_top("hs \u{00b7} scripted \u{00b7} $0.0000", 60, true);
    assert!(s.contains('\u{256d}'), "top-left corner: {s:?}");
    assert!(s.contains('\u{256e}'), "top-right corner: {s:?}");
    assert!(s.contains("hs \u{00b7} scripted \u{00b7} $0.0000"), "label: {s:?}");
    assert!(s.contains("\x1b["), "colored frame carries ANSI: {s:?}");
    assert_eq!(visible_width(&s), 60, "visible width excludes escapes");
}

// R2: plain mode is ANSI-free at the same visible width.
#[test]
fn r2_composer_top_plain() {
    let s = composer_top("hs", 40, false);
    assert!(!s.contains("\x1b["), "plain frame stays ANSI-free: {s:?}");
    assert_eq!(visible_width(&s), 40);
    assert!(s.contains("hs"));
}

// R3: the bottom border mirrors the top at the same width.
#[test]
fn r3_composer_bottom() {
    let s = composer_bottom(60, true);
    assert!(s.contains('\u{2570}'), "bottom-left corner: {s:?}");
    assert!(s.contains('\u{256f}'), "bottom-right corner: {s:?}");
    assert_eq!(visible_width(&s), 60);
}

// R4: a section separator rule (layout density): label mid-line,
// exact width, dim on a terminal.
#[test]
fn r4_separator_rule() {
    let s = separator("result", 60, true);
    assert!(s.contains("result"));
    assert!(s.contains('\u{2500}'), "rule glyph: {s:?}");
    assert_eq!(visible_width(&s), 60);
    assert!(s.contains("\x1b["), "dim rule carries ANSI: {s:?}");
}

// R5: the composer prompt carries the box's left bar.
#[test]
fn r5_editor_prompt() {
    assert_eq!(EDITOR_PROMPT, "\u{2502} hs> ");
}
