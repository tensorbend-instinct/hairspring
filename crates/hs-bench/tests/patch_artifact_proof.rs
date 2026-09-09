//! PATCH APPLICATION ARTIFACT HYGIENE - .hs-eval.patch must never be left
//! inside the workspace.
//!
//! `apply_model_patch` used to drop its temp patch file at
//! <ws>/.hs-eval.patch. The file is harness machinery, but it landed inside
//! the agent-visible tree: agents saw it, experimented on it, and - worse -
//! if a crash interrupted cleanup after the file entered the git index, the
//! candidate diff grew a spurious "delete .hs-eval.patch" hunk that could
//! break checker application (observed live on conan-17302 / cfn-lint-3767,
//! 2026-09-07). Falsifiable: if the temp file lands anywhere inside the
//! workspace, agents can see and corrupt harness state.

use hs_bench::apply_model_patch;
use std::path::Path;
use std::process::Command;

fn git(ws: &Path, args: &[&str]) {
    let out = Command::new("git")
        .args(args)
        .current_dir(ws)
        .output()
        .expect("git runs");
    assert!(out.status.success(), "git {args:?}: {out:?}");
}

fn make_ws() -> std::path::PathBuf {
    let ws = std::env::temp_dir().join(format!(
        "hs-artifact-proof-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&ws).unwrap();
    std::fs::write(ws.join("code.txt"), "broken\n").unwrap();
    git(&ws, &["init", "-q"]);
    git(&ws, &["add", "-A"]);
    git(
        &ws,
        &[
            "-c",
            "user.email=b@b",
            "-c",
            "user.name=b",
            "commit",
            "-qm",
            "base",
        ],
    );
    ws
}

#[test]
fn apply_leaves_no_artifact_in_workspace() {
    let ws = make_ws();
    let patch = "--- a/code.txt\n+++ b/code.txt\n@@ -1 +1 @@\n-broken\n+fixed\n";
    let res = apply_model_patch(&ws, patch).expect("apply machinery works");
    assert!(
        matches!(res, hs_bench::ApplyResult::Applied),
        "valid patch applies"
    );
    assert_eq!(
        std::fs::read_to_string(ws.join("code.txt")).unwrap(),
        "fixed\n",
        "patch content actually applied"
    );
    assert!(
        !ws.join(".hs-eval.patch").exists(),
        "harness temp patch file must not remain inside the workspace"
    );
    // and nothing stray is visible to git either (untracked files included)
    let status = Command::new("git")
        .args(["status", "--porcelain"])
        .current_dir(&ws)
        .output()
        .unwrap();
    let text = String::from_utf8_lossy(&status.stdout);
    assert!(
        !text.contains("hs-eval"),
        "no hs-eval artifact may appear in git status: {text}"
    );
    let _ = std::fs::remove_dir_all(&ws);
}

#[test]
fn failed_apply_also_leaves_no_artifact() {
    let ws = make_ws();
    let bad = "--- a/code.txt\n+++ b/code.txt\n@@ -1 +1 @@\n-different\n+fixed\n";
    let res = apply_model_patch(&ws, bad).expect("apply machinery works");
    assert!(
        matches!(res, hs_bench::ApplyResult::NoApply(_)),
        "context mismatch is NoApply, not a panic"
    );
    assert!(
        !ws.join(".hs-eval.patch").exists(),
        "even a failed apply must not leave the artifact behind"
    );
    let _ = std::fs::remove_dir_all(&ws);
}
