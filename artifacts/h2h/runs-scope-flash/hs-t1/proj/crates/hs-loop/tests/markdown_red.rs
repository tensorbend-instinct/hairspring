//! REPL UI gap #4 (Eric 2026-09-08: "just fix the gaps", SOTA-REPL UI
//! investigation): pi/omp render the model's prose as MARKDOWN while it
//! streams - headers bold, code spans tinted, fenced blocks set off,
//! lists bulleted. hs-repl prints raw deltas: "**bold**" stays literal
//! asterisks on screen.
//!
//! Contract: a streaming markdown renderer (`MarkdownStreamer`) that eats
//! deltas of ANY chunking (a token can split mid-construct) and emits
//! semantically colored terminal text: constructs are styled, markers
//! are consumed, and the rendered output is identical no matter how the
//! input was split. Plain mode (piped) strips markers without ANSI.

use hs_loop::uipaint::MarkdownStreamer;

fn render_all(chunks: &[&str], color: bool) -> String {
    let mut out: Vec<u8> = Vec::new();
    let mut s = MarkdownStreamer::new(color);
    for c in chunks {
        s.push(c, &mut out);
    }
    s.finish(&mut out);
    String::from_utf8(out).unwrap()
}

const DOC: &str = "# Title\nSome **bold** and `code` here.\n```\n**not bold**\n```\n- item one\n";

// R1: markdown constructs get semantic color; markers are consumed.
#[test]
fn r1_constructs_get_semantic_color() {
    let s = render_all(&[DOC], true);
    assert!(s.contains("\x1b["), "colored render carries ANSI: {s:?}");
    assert!(s.contains("Title"), "header text survives: {s:?}");
    assert!(!s.contains("# Title"), "header marker consumed: {s:?}");
    assert!(s.contains("**not bold**"), "fenced content stays literal: {s:?}");
    // inline markers consumed outside fences
    let inline_line = s.lines().find(|l| l.contains("bold") && !l.contains("not bold")).unwrap();
    assert!(!inline_line.contains("**"), "bold markers consumed: {inline_line:?}");
    assert!(!inline_line.contains('`'), "code markers consumed: {inline_line:?}");
    assert!(inline_line.contains("bold") && inline_line.contains("code"));
    // bold is actually bold (SGR 1) around the word
    assert!(inline_line.contains("\x1b[1m") || inline_line.contains(";1m"), "bold word styled: {inline_line:?}");
}

// R2: chunk-split invariance - the SAME document renders byte-identical
// whether fed whole, in 1-byte drips, or split mid-construct.
#[test]
fn r2_chunk_split_invariance() {
    let whole = render_all(&[DOC], true);
    let drips: Vec<&str> = DOC.split_inclusive(|_| true).collect();
    let dripped = render_all(&drips, true);
    assert_eq!(whole, dripped, "1-byte drips must render identically");
    // split inside a **bold** construct and inside a fence opener
    let split_a = DOC.find("**bo").unwrap();
    let a = render_all(&[&DOC[..=split_a], &DOC[split_a + 1..]], true);
    assert_eq!(whole, a, "split mid-bold renders identically");
    let split_b = DOC.find("```").unwrap();
    let b = render_all(&[&DOC[..split_b + 2], &DOC[split_b + 2..]], true);
    assert_eq!(whole, b, "split mid-fence renders identically");
}

// R3: plain mode strips markers, emits zero ANSI, preserves content.
#[test]
fn r3_plain_mode_strips_markers_no_ansi() {
    let s = render_all(&[DOC], false);
    assert!(!s.contains("\x1b["), "plain render stays ANSI-free: {s:?}");
    assert!(s.contains("Title") && s.contains("bold") && s.contains("code"));
    assert!(!s.contains("# Title"), "header marker stripped: {s:?}");
    let inline_line = s.lines().find(|l| l.contains("bold") && !l.contains("not bold")).unwrap();
    assert!(!inline_line.contains("**") && !inline_line.contains('`'));
    assert!(s.contains("**not bold**"), "fenced content literal even in plain mode");
    assert!(!s.contains("```"), "fence markers never printed: {s:?}");
}

// R4: a partial final line (no trailing newline) still renders at
// finish() - the model's last tokens must not vanish.
#[test]
fn r4_partial_line_flushes_at_finish() {
    let s = render_all(&["tail with **bold**"], true);
    assert!(s.contains("bold"), "partial line content rendered: {s:?}");
    assert!(!s.contains("**"), "partial line markers consumed: {s:?}");
}
