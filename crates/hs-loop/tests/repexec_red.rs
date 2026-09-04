//! RED contract tests for repo.exec (Eric's 13:50 directive: the model must be
//! able to run lint/tests on its own patch before answer.write).
//! API under test (does not exist yet): hs_loop::repexec::run(ws, answer_path,
//! command, allowlist, timeout_secs) -> serde_json::Value
//! Contract:
//! - scratch only: the live workspace is NEVER mutated (checker semantics
//!   unchanged); the current answer patch is applied to a scratch copy.
//! - allowlist: only configured command prefixes execute; anything else is
//!   feedback, never a shell.
//! - apply pre-flight: a patch that does not apply comes back as a clean
//!   applied=false result (the 3/3 framing-failure catch from the A/B/C).
//! - timeout is enforced and reported.
//! - no answer yet: clean "no patch to test" feedback, not an error.

use std::io::Write;
use std::path::Path;

/// Minimal git workspace: one file, one commit.
fn mk_ws() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    let ws = dir.path();
    std::fs::write(ws.join("app.py"), "x = 1\n").unwrap();
    let git = |args: &[&str]| {
        std::process::Command::new("git")
            .args(args)
            .current_dir(ws)
            .env("GIT_AUTHOR_NAME", "t")
            .env("GIT_AUTHOR_EMAIL", "t@t")
            .env("GIT_COMMITTER_NAME", "t")
            .env("GIT_COMMITTER_EMAIL", "t@t")
            .output()
            .unwrap()
    };
    git(&["init", "-q"]);
    git(&["add", "."]);
    git(&["commit", "-qm", "init"]);
    dir
}

const PATCH_OK: &str = "```diff\ndiff --git a/app.py b/app.py\n--- a/app.py\n+++ b/app.py\n@@ -1 +1 @@\n-x = 1\n+x = 2\n```\n";
const PATCH_BAD: &str = "```diff\n@@ -1 +1 @@\n-x = 1\n+x = 2\n```\n";

fn allow() -> Vec<String> {
    vec!["python3 -m pytest".into(), "git apply --check".into(), "cat".into()]
}

#[test]
fn exec_applies_patch_in_scratch_and_runs_command() {
    let ws = mk_ws();
    let ans = ws.path().join("answer.txt");
    std::fs::write(&ans, PATCH_OK).unwrap();
    let r = hs_loop::repexec::run(ws.path(), &ans, "cat app.py", &allow(), 30);
    assert_eq!(r["applied"], true, "patch should apply: {r}");
    assert_eq!(r["exit_code"], 0);
    assert!(r["stdout"].as_str().unwrap().contains("x = 2"), "scratch sees the patch: {r}");
    // live workspace untouched
    assert_eq!(std::fs::read_to_string(ws.path().join("app.py")).unwrap(), "x = 1\n");
}

#[test]
fn exec_preflight_reports_unappliable_patch_without_running() {
    let ws = mk_ws();
    let ans = ws.path().join("answer.txt");
    std::fs::write(&ans, PATCH_BAD).unwrap();
    let r = hs_loop::repexec::run(ws.path(), &ans, "cat app.py", &allow(), 30);
    assert_eq!(r["applied"], false);
    assert!(r["apply_error"].as_str().unwrap().len() > 3, "names the apply failure: {r}");
    assert!(r.get("exit_code").is_none() || r["exit_code"].is_null(), "command must not run");
}

#[test]
fn exec_rejects_non_allowlisted_command() {
    let ws = mk_ws();
    let ans = ws.path().join("answer.txt");
    std::fs::write(&ans, PATCH_OK).unwrap();
    let r = hs_loop::repexec::run(ws.path(), &ans, "rm -rf /", &allow(), 30);
    assert!(r["$error"].as_str().unwrap().contains("allowlist"), "{r}");
    assert!(ws.path().join("app.py").exists());
}

#[test]
fn exec_enforces_timeout() {
    let ws = mk_ws();
    let ans = ws.path().join("answer.txt");
    std::fs::write(&ans, PATCH_OK).unwrap();
    let mut al = allow();
    al.push("sleep".into());
    let t0 = std::time::Instant::now();
    let r = hs_loop::repexec::run(ws.path(), &ans, "sleep 30", &al, 2);
    assert!(t0.elapsed().as_secs() < 15, "timeout must actually fire");
    assert_eq!(r["timed_out"], true, "{r}");
}

#[test]
fn exec_without_answer_is_clean_feedback() {
    let ws = mk_ws();
    let ans = ws.path().join("answer.txt");
    let r = hs_loop::repexec::run(ws.path(), &ans, "cat app.py", &allow(), 30);
    assert_eq!(r["applied"], false);
    assert!(r["note"].as_str().unwrap_or("").contains("no patch"), "{r}");
}

#[test]
fn exec_cleans_up_scratch() {
    let ws = mk_ws();
    let ans = ws.path().join("answer.txt");
    std::fs::write(&ans, PATCH_OK).unwrap();
    let before = hs_loop::repexec::run(ws.path(), &ans, "cat app.py", &allow(), 30);
    assert_eq!(before["applied"], true);
    let wt = std::process::Command::new("git")
        .args(["worktree", "list", "--porcelain"])
        .current_dir(ws.path())
        .output()
        .unwrap();
    let n = String::from_utf8_lossy(&wt.stdout).matches("worktree ").count();
    assert_eq!(n, 1, "scratch worktree must be removed: {}", String::from_utf8_lossy(&wt.stdout));
}
