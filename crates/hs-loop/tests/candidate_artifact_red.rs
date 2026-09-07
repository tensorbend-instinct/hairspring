//! RED (2026-09-07): the editapply candidate must never carry the harness
//! eval artifact into the model-visible cumulative diff.
//!
//! Live evidence (conan-17302, fixed-build rerun): the pre-6398fc8f eval
//! flow stranded .hs-eval.patch inside the ws, where it was COMMITTED into
//! the ws HEAD. The candidate worktree branches from that HEAD, so the
//! artifact is tracked there; read_cumulative then deleted the worktree copy
//! and ran `git add -A`, staging a spurious "delete .hs-eval.patch" hunk.
//! The rerun agent inherited that poisoned candidate and burned ~30 steps
//! fighting the phantom deletion. Falsifiable: apply one real edit through
//! editapply on a ws whose HEAD tracks .hs-eval.patch; the cumulative diff
//! must contain ONLY the real edit.

use std::path::Path;
use std::process::Command;

fn git(dir: &Path, args: &[&str]) {
    let out = Command::new("git")
        .args(args)
        .current_dir(dir)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

const EDIT: &str =
    "diff --git a/real.py b/real.py\n--- a/real.py\n+++ b/real.py\n@@ -1 +1 @@\n-x = 1\n+x = 2\n";

/// A workspace whose HEAD carries the stranded harness artifact, committed
/// (the conan-17302 contamination shape left by the old eval flow).
fn mk_contaminated_ws() -> tempfile::TempDir {
    let d = tempfile::tempdir().unwrap();
    git(d.path(), &["init", "-q"]);
    git(d.path(), &["config", "user.email", "t@t"]);
    git(d.path(), &["config", "user.name", "t"]);
    std::fs::write(d.path().join(".hs-eval.patch"), "diff --git ARTIFACT\n").unwrap();
    std::fs::write(d.path().join("real.py"), "x = 1\n").unwrap();
    git(d.path(), &["add", "-A"]);
    git(d.path(), &["commit", "-qm", "base"]);
    d
}

#[test]
fn cumulative_diff_never_contains_eval_artifact() {
    let ws = mk_contaminated_ws();
    let r = hs_loop::editapply::apply(ws.path(), EDIT);
    assert_eq!(r["applied"], true, "apply failed: {r}");
    let cd = r["cumulative_diff"].as_str().unwrap();
    assert!(
        cd.contains("+x = 2"),
        "real change missing from cumulative diff:\n{cd}"
    );
    assert!(
        !cd.contains("hs-eval"),
        "harness artifact leaked into cumulative diff:\n{cd}"
    );
}

#[test]
fn artifact_state_stable_across_calls() {
    let ws = mk_contaminated_ws();
    let r = hs_loop::editapply::apply(ws.path(), EDIT);
    assert_eq!(r["applied"], true, "apply failed: {r}");
    // The agent-visible candidate worktree must hold the artifact with its
    // HEAD content - not a deletion, not an edit target (17302's agent found
    // "no such file in candidate" and started experimenting on it).
    const EDIT2: &str = "diff --git a/real.py b/real.py\n--- a/real.py\n+++ b/real.py\n@@ -1 +1 @@\n-x = 2\n+x = 3\n";
    let r2 = hs_loop::editapply::apply(ws.path(), EDIT2);
    assert_eq!(r2["applied"], true, "second apply failed: {r2}");
    let cd = r2["cumulative_diff"].as_str().unwrap();
    assert!(
        !cd.contains("hs-eval"),
        "artifact state destabilized across calls:\n{cd}"
    );
}

/// Control: an UNTRACKED stray copy in the ws worktree (fresh-binary shape)
/// must stay out of the cumulative diff too. Green before and after the fix.
#[test]
fn untracked_artifact_copy_stays_out_of_diff() {
    let d = tempfile::tempdir().unwrap();
    git(d.path(), &["init", "-q"]);
    git(d.path(), &["config", "user.email", "t@t"]);
    git(d.path(), &["config", "user.name", "t"]);
    std::fs::write(d.path().join("real.py"), "x = 1\n").unwrap();
    git(d.path(), &["add", "-A"]);
    git(d.path(), &["commit", "-qm", "base"]);
    std::fs::write(d.path().join(".hs-eval.patch"), "stray\n").unwrap();
    let r = hs_loop::editapply::apply(d.path(), EDIT);
    assert_eq!(r["applied"], true, "apply failed: {r}");
    let cd = r["cumulative_diff"].as_str().unwrap();
    assert!(
        !cd.contains("hs-eval"),
        "untracked artifact leaked into cumulative diff:\n{cd}"
    );
}
