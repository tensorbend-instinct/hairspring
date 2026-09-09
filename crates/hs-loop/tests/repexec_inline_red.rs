//! RED contract tests for repo.exec INLINE DIFF (T4: test-before-first-submit,
//! defect S3). Before this, repo.exec refused to run until the model had
//! written an answer file - the model could not test a candidate patch, which
//! forced blind answer.write submissions. New API:
//!   `hs_loop::repexec::run_sandboxed_with_diff(ws`, `diff_text`, command, timeout)
//! and the repo.exec plugin accepts args.diff (fenced or raw unified diff)
//! as an alternative to args.path / `HS_SWE_ANSWER`. Sandbox semantics are
//! identical: scratch worktree, bwrap, live ws untouched.

use std::io::Write;
use std::process::{Command, Stdio};

fn mk_ws() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    let ws = dir.path();
    std::fs::write(ws.join("app.py"), "x = 1\n").unwrap();
    let git = |args: &[&str]| {
        assert!(Command::new("git")
            .args(args)
            .current_dir(ws)
            .env("GIT_AUTHOR_NAME", "t")
            .env("GIT_AUTHOR_EMAIL", "t@t")
            .env("GIT_COMMITTER_NAME", "t")
            .env("GIT_COMMITTER_EMAIL", "t@t")
            .status()
            .unwrap()
            .success());
    };
    git(&["init", "-q"]);
    git(&["add", "."]);
    git(&["commit", "-qm", "init"]);
    dir
}

const RAW_DIFF: &str =
    "diff --git a/app.py b/app.py\n--- a/app.py\n+++ b/app.py\n@@ -1 +1 @@\n-x = 1\n+x = 2\n";
const FENCED_DIFF: &str = "```diff\ndiff --git a/app.py b/app.py\n--- a/app.py\n+++ b/app.py\n@@ -1 +1 @@\n-x = 1\n+x = 2\n```\n";

#[test]
fn inline_diff_runs_before_first_answer_write() {
    let d = mk_ws();
    // no answer file exists anywhere; the model passes the patch inline
    let r = hs_loop::repexec::run_sandboxed_with_diff(d.path(), RAW_DIFF, "cat app.py", 30);
    assert_eq!(r["applied"], true, "{r}");
    assert_eq!(r["exit_code"], 0, "{r}");
    assert!(r["stdout"].as_str().unwrap().contains("x = 2"), "{r}");
    // live ws untouched
    assert_eq!(
        std::fs::read_to_string(d.path().join("app.py")).unwrap(),
        "x = 1\n"
    );
}

#[test]
fn inline_diff_accepts_fenced_or_raw() {
    let d = mk_ws();
    let r = hs_loop::repexec::run_sandboxed_with_diff(d.path(), FENCED_DIFF, "cat app.py", 30);
    assert_eq!(r["applied"], true, "fenced: {r}");
    assert!(
        r["stdout"].as_str().unwrap().contains("x = 2"),
        "fenced: {r}"
    );
}

#[test]
fn inline_diff_bad_patch_is_clean_feedback_not_machinery_error() {
    let d = mk_ws();
    let r = hs_loop::repexec::run_sandboxed_with_diff(d.path(), "this is not a diff", "true", 30);
    assert_eq!(r["applied"], false, "{r}");
    assert!(
        r.get("$error").is_none(),
        "machinery must not fail on model input: {r}"
    );
    assert!(
        r.get("note").is_some() || r.get("apply_error").is_some(),
        "feedback: {r}"
    );
}

fn serve_roundtrip(bin: &str, ws: &std::path::Path, req: &str) -> serde_json::Value {
    let mut p = Command::new(bin)
        .current_dir(ws)
        .env("HS_SWE_WORKSPACE", ws)
        .env_remove("HS_SWE_ANSWER")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let mut stdin = p.stdin.take().unwrap();
    let req = req.to_string();
    std::thread::spawn(move || {
        stdin.write_all(req.as_bytes()).unwrap();
        drop(stdin);
    });
    let out = p.wait_with_output().unwrap();
    let line = String::from_utf8_lossy(&out.stdout);
    let resp = line
        .lines()
        .nth(1)
        .unwrap_or_else(|| panic!("describe + call responses: {line}"));
    serde_json::from_str(resp).unwrap()
}

#[test]
fn plugin_accepts_args_diff_without_any_answer_path() {
    let d = mk_ws();
    let bin = env!("CARGO_BIN_EXE_hs-plugin-repoexec");
    // describe, then a call with args.diff and NO path; HS_SWE_ANSWER unset
    let req = "{\"id\":1,\"method\":\"describe\",\"params\":{}}\n{\"id\":2,\"method\":\"tool.call\",\"params\":{\"args\":{\"command\":\"cat app.py\",\"diff\":\"diff --git a/app.py b/app.py\\n--- a/app.py\\n+++ b/app.py\\n@@ -1 +1 @@\\n-x = 1\\n+x = 2\\n\"}}}\n";
    let resp = serve_roundtrip(bin, d.path(), req);
    let r = &resp["result"];
    assert_eq!(r["applied"], true, "{resp}");
    assert!(r["stdout"].as_str().unwrap().contains("x = 2"), "{resp}");
}

#[test]
fn plugin_inline_diff_beats_stale_answer_path_when_both_given() {
    // args.diff is the candidate being tested NOW; a previously written
    // answer file must not shadow it.
    let d = mk_ws();
    let stale = "x = 99\n";
    std::fs::write(d.path().join("answer.txt"),
        format!("```diff\ndiff --git a/app.py b/app.py\n--- a/app.py\n+++ b/app.py\n@@ -1 +1 @@\n-x = 1\n+{stale}```\n")).unwrap();
    let bin = env!("CARGO_BIN_EXE_hs-plugin-repoexec");
    let req = format!("{{\"id\":1,\"method\":\"describe\",\"params\":{{}}}}\n{{\"id\":2,\"method\":\"tool.call\",\"params\":{{\"args\":{{\"command\":\"cat app.py\",\"path\":\"{}\",\"diff\":\"diff --git a/app.py b/app.py\\n--- a/app.py\\n+++ b/app.py\\n@@ -1 +1 @@\\n-x = 1\\n+x = 2\\n\"}}}}}}\n", d.path().join("answer.txt").display());
    let resp = serve_roundtrip(bin, d.path(), &req);
    let r = &resp["result"];
    assert_eq!(r["applied"], true, "{resp}");
    assert!(
        r["stdout"].as_str().unwrap().contains("x = 2"),
        "inline diff must win: {resp}"
    );
    assert!(
        !r["stdout"].as_str().unwrap().contains("x = 99"),
        "stale answer must not shadow: {resp}"
    );
}
