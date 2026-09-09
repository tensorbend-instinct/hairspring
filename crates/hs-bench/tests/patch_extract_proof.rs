//! GATE 8 BENCHMARK PREP 3 - patch extraction from model completions.
//!
//! Real models wrap diffs in prose and fences. The extractor must find the
//! unified diff, reject prose-only completions (`NoApply` upstream), and never
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

// 2026-09-07 octodns-1298: extract_patch's body.trim() ate the diff's
// trailing blank-line context (a context line that is a single space),
// turning a VALID git-produced diff into "error: corrupt patch at line N"
// on BOTH the checker and the repo.exec paths. Falsifiable: a real git
// diff whose last hunk ends on a blank context line must survive
// extraction byte-intact and apply cleanly.
#[test]
fn preserves_trailing_blank_context_line() {
    let ws = std::env::temp_dir().join(format!(
        "hs-extract-blank-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&ws).unwrap();
    let git = |args: &[&str]| {
        let o = std::process::Command::new("git")
            .args(args)
            .current_dir(&ws)
            .output()
            .unwrap();
        assert!(
            o.status.success(),
            "git {:?}: {}",
            args,
            String::from_utf8_lossy(&o.stderr)
        );
        o
    };
    git(&["init", "-q"]);
    // trailing blank line in the file -> the hunk's final context line is
    // a single space, the exact line trim() destroys
    std::fs::write(ws.join("code.txt"), "a\nb\nX\n\n").unwrap();
    git(&["add", "-A"]);
    git(&[
        "-c",
        "user.email=b@b",
        "-c",
        "user.name=b",
        "commit",
        "-qm",
        "base",
    ]);
    std::fs::write(ws.join("code.txt"), "a\nb\nY\n\n").unwrap();
    let raw = String::from_utf8(git(&["diff"]).stdout).unwrap();
    assert!(
        raw.ends_with(" \n"),
        "fixture sanity: git diff ends in a blank context line"
    );

    let extracted = extract_patch(&raw).expect("bare git diff must extract");
    assert!(
        extracted.ends_with(" \n"),
        "trailing blank context line must survive extraction; got {extracted:?}"
    );

    // and the extracted patch must apply cleanly to the pristine base
    git(&["checkout", "--", "."]);
    std::fs::write(ws.join("p.patch"), &extracted).unwrap();
    let check = std::process::Command::new("git")
        .args(["apply", "--check", "p.patch"])
        .current_dir(&ws)
        .output()
        .unwrap();
    assert!(
        check.status.success(),
        "extracted patch must apply: {}",
        String::from_utf8_lossy(&check.stderr)
    );
    let _ = std::fs::remove_dir_all(&ws);
}
