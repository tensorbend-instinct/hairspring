//! RED contract tests for the D5 mission tools edit.apply and notes.scratch.
//!
//! edit.apply: persistent candidate worktree per workspace. The model applies
//! incremental unified diffs to a CANDIDATE (never the live ws) and gets the
//! cumulative diff vs base back on every call - the submit path can grade the
//! candidate without the model ever re-serializing its whole patch.
//!   hs_loop::editapply::{apply, cumulative_diff, reset}(ws, ...) -> Value
//!
//! notes.scratch: model-writable persistent notes (op write/append/read),
//! stored at HS_SCRATCH_FILE, surviving across calls and plugin restarts.

use std::io::Write;
use std::process::{Command, Stdio};

fn mk_ws() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    let ws = dir.path();
    std::fs::write(ws.join("app.py"), "x = 1\n").unwrap();
    std::fs::write(ws.join("lib.py"), "def f():\n    return 1\n").unwrap();
    let git = |args: &[&str]| {
        assert!(Command::new("git").args(args).current_dir(ws)
            .env("GIT_AUTHOR_NAME", "t").env("GIT_AUTHOR_EMAIL", "t@t")
            .env("GIT_COMMITTER_NAME", "t").env("GIT_COMMITTER_EMAIL", "t@t")
            .status().unwrap().success());
    };
    git(&["init", "-q"]);
    git(&["add", "."]);
    git(&["commit", "-qm", "init"]);
    dir
}

const D1: &str = "diff --git a/app.py b/app.py\n--- a/app.py\n+++ b/app.py\n@@ -1 +1 @@\n-x = 1\n+x = 2\n";
const D2: &str = "diff --git a/lib.py b/lib.py\n--- a/lib.py\n+++ b/lib.py\n@@ -1,2 +1,2 @@\n def f():\n-    return 1\n+    return 2\n";

#[test]
fn edit_apply_returns_cumulative_diff_and_live_ws_untouched() {
    let d = mk_ws();
    let r = hs_loop::editapply::apply(d.path(), D1);
    assert_eq!(r["applied"], true, "{r}");
    let cd = r["cumulative_diff"].as_str().unwrap();
    assert!(cd.contains("+x = 2"), "{cd}");
    assert_eq!(std::fs::read_to_string(d.path().join("app.py")).unwrap(), "x = 1\n");
    hs_loop::editapply::reset(d.path());
}

#[test]
fn edit_apply_accumulates_across_calls_and_survives_reentry() {
    let d = mk_ws();
    let r1 = hs_loop::editapply::apply(d.path(), D1);
    assert_eq!(r1["applied"], true, "{r1}");
    // simulate a fresh plugin process: no in-memory state may be required
    let r2 = hs_loop::editapply::apply(d.path(), D2);
    assert_eq!(r2["applied"], true, "{r2}");
    let cd = r2["cumulative_diff"].as_str().unwrap();
    assert!(cd.contains("+x = 2") && cd.contains("+    return 2"), "cumulative: {cd}");
    // cumulative_diff is also readable without a new apply
    let only = hs_loop::editapply::cumulative_diff(d.path());
    assert_eq!(only["has_candidate"], true, "{only}");
    assert!(only["cumulative_diff"].as_str().unwrap().contains("+x = 2"), "{only}");
    hs_loop::editapply::reset(d.path());
    let gone = hs_loop::editapply::cumulative_diff(d.path());
    assert_eq!(gone["has_candidate"], false, "{gone}");
}

#[test]
fn edit_apply_bad_diff_is_clean_feedback_and_candidate_unchanged() {
    let d = mk_ws();
    let r1 = hs_loop::editapply::apply(d.path(), D1);
    assert_eq!(r1["applied"], true, "{r1}");
    let r2 = hs_loop::editapply::apply(d.path(), "diff --git a/app.py b/app.py\n--- a/app.py\n+++ b/app.py\n@@ -1 +1 @@\n-x = WRONG\n+x = 9\n");
    assert_eq!(r2["applied"], false, "{r2}");
    assert!(r2.get("apply_error").is_some(), "{r2}");
    // earlier good work survives the failed apply
    let cd = hs_loop::editapply::cumulative_diff(d.path());
    assert!(cd["cumulative_diff"].as_str().unwrap().contains("+x = 2"), "{cd}");
    hs_loop::editapply::reset(d.path());
}

fn serve_roundtrip(bin: &str, envs: &[(&str, &std::path::Path)], reqs: &str) -> Vec<serde_json::Value> {
    let mut c = Command::new(bin);
    for (k, v) in envs { c.env(k, v); }
    let mut p = c.stdin(Stdio::piped()).stdout(Stdio::piped()).spawn().unwrap();
    let mut stdin = p.stdin.take().unwrap();
    let reqs = reqs.to_string();
    std::thread::spawn(move || { stdin.write_all(reqs.as_bytes()).unwrap(); drop(stdin); });
    let out = p.wait_with_output().unwrap();
    String::from_utf8_lossy(&out.stdout).lines()
        .map(|l| serde_json::from_str(l).unwrap()).collect()
}

#[test]
fn notes_scratch_write_append_read_across_restarts() {
    let dir = tempfile::tempdir().unwrap();
    let notes = dir.path().join("notes.md");
    let bin = env!("CARGO_BIN_EXE_hs-plugin-notescratch");
    let envs = [("HS_SCRATCH_FILE", notes.as_path())];
    // process 1: write
    let r = serve_roundtrip(bin, &envs,
        "{\"id\":1,\"method\":\"describe\",\"params\":{}}\n{\"id\":2,\"method\":\"tool.call\",\"params\":{\"args\":{\"op\":\"write\",\"content\":\"hypothesis: parser off-by-one\\n\"}}}\n");
    assert!(r[1]["result"]["ok"].as_bool().unwrap(), "{r:?}");
    // process 2 (fresh): append + read back both lines
    let r = serve_roundtrip(bin, &envs,
        "{\"id\":1,\"method\":\"tool.call\",\"params\":{\"args\":{\"op\":\"append\",\"content\":\"test: cargo test -p parser\\n\"}}}\n{\"id\":2,\"method\":\"tool.call\",\"params\":{\"args\":{\"op\":\"read\"}}}\n");
    assert!(r[0]["result"]["ok"].as_bool().unwrap(), "{r:?}");
    let body = r[1]["result"]["content"].as_str().unwrap();
    assert!(body.contains("hypothesis: parser off-by-one"), "{body}");
    assert!(body.contains("test: cargo test -p parser"), "{body}");
    // read on empty notes is clean feedback, not an error
    let notes2 = dir.path().join("empty.md");
    let r = serve_roundtrip(bin, &[("HS_SCRATCH_FILE", notes2.as_path())],
        "{\"id\":1,\"method\":\"tool.call\",\"params\":{\"args\":{\"op\":\"read\"}}}\n");
    assert!(r[0]["result"].get("$error").is_none(), "{r:?}");
    assert_eq!(r[0]["result"]["content"].as_str().unwrap(), "");
}
