//! GATE 8 BENCHMARK PREP 3 - patch extraction from model completions.
//!
//! Real models wrap diffs in prose and fences. The extractor must find the
//! unified diff, reject prose-only completions (NoApply upstream), and never
//! invent a patch. Falsifiable: if prose without a diff yields a patch, the
//! eval could credit garbage; if a fenced diff is missed, good missions die.

use hs_bench::extract_patch;

#[test]
fn extracts_fenced_diff() {
    let completion = "I found the bug.\n\n```diff\n--- a/code.txt\n+++ b/code.txt\n@@ -1 +1 @@\n-broken\n+fixed\n```\n\nThat should do it.";
    let p = extract_patch(completion);
    assert!(p.is_some(), "fenced diff must be found");
    assert!(p.unwrap().contains("+fixed"));
}

#[test]
fn extracts_bare_unified_diff() {
    let completion = "Here:\n--- a/code.txt\n+++ b/code.txt\n@@ -1 +1 @@\n-broken\n+fixed\n";
    let p = extract_patch(completion);
    assert!(p.is_some(), "bare diff must be found");
}

#[test]
fn prose_only_yields_no_patch() {
    assert!(extract_patch("I think the bug is in the parser, but I cannot decide.").is_none());
    assert!(extract_patch("").is_none());
    // a fence without diff content is not a patch
    assert!(extract_patch("```\njust some code, no diff headers\n```").is_none());
}

#[test]
fn picks_the_diff_not_other_fences() {
    let completion = "```rust\nfn main() {}\n```\n\nThe fix:\n```diff\n--- a/code.txt\n+++ b/code.txt\n@@ -1 +1 @@\n-broken\n+fixed\n```";
    let p = extract_patch(completion).unwrap();
    assert!(p.contains("--- a/code.txt"));
    assert!(!p.contains("fn main"), "must not grab the rust fence");
}
