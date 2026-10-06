//! RED contract tests: edit.apply takes search/replace blocks, not diffs.
//!
//! Measured driver (2026-09-05, conan-17092 A6/B6 runs): 6 of 6 model-written
//! unified diffs failed git apply - hunk-header line-count arithmetic errors
//! (dominant class) and truncated patch tails. Every failed patch was
//! semantically correct; the FORMAT was the failure. New contract:
//!   args.edits = [{"path","old","new"}, ...] applied in order, all-or-nothing
//!   exact-unique match required; whitespace-tolerant fallback on a unique
//!   fuzzy match; failures name the block and the reason and leave the
//!   candidate exactly as it was. Candidate worktree + cumulative diff
//!   machinery unchanged. The raw diff arg is retired with a steering error.
//!
//!   `hs_loop::editapply::{EditBlock`, `apply_blocks}(ws`, &[`EditBlock`]) -> Value

use std::io::Write;
use std::process::{Command, Stdio};

fn mk_ws() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    let ws = dir.path();
    std::fs::write(ws.join("app.py"), "x = 1\n").unwrap();
    std::fs::write(ws.join("lib.py"), "def f():\n    return 1\n").unwrap();
    std::fs::write(ws.join("dup.py"), "a = 1\nb = 2\na = 1\n").unwrap();
    std::fs::write(ws.join("trail.py"), "y = 3   \n").unwrap();
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

fn blk(path: &str, old: &str, new: &str) -> hs_loop::editapply::EditBlock {
    hs_loop::editapply::EditBlock {
        path: path.into(),
        old: old.into(),
        new: new.into(),
    }
}

#[test]
fn splice_single_block_applies_and_live_ws_untouched() {
    let d = mk_ws();
    let r = hs_loop::editapply::apply_blocks(d.path(), &[blk("app.py", "x = 1\n", "x = 2\n")]);
    assert_eq!(r["applied"], true, "{r}");
    let cd = r["cumulative_diff"].as_str().unwrap();
    assert!(cd.contains("+x = 2"), "{cd}");
    assert_eq!(
        std::fs::read_to_string(d.path().join("app.py")).unwrap(),
        "x = 1\n"
    );
    let _ = hs_loop::editapply::reset(d.path());
}

#[test]
fn splice_accumulates_across_calls_and_survives_reentry() {
    let d = mk_ws();
    let r1 = hs_loop::editapply::apply_blocks(d.path(), &[blk("app.py", "x = 1\n", "x = 2\n")]);
    assert_eq!(r1["applied"], true, "{r1}");
    // fresh plugin process: no in-memory state may be required
    let r2 = hs_loop::editapply::apply_blocks(
        d.path(),
        &[blk("lib.py", "    return 1\n", "    return 2\n")],
    );
    assert_eq!(r2["applied"], true, "{r2}");
    let cd = r2["cumulative_diff"].as_str().unwrap();
    assert!(
        cd.contains("+x = 2") && cd.contains("+    return 2"),
        "cumulative: {cd}"
    );
    let only = hs_loop::editapply::cumulative_diff(d.path());
    assert_eq!(only["has_candidate"], true, "{only}");
    let _ = hs_loop::editapply::reset(d.path());
    assert_eq!(
        hs_loop::editapply::cumulative_diff(d.path())["has_candidate"],
        false
    );
}

#[test]
fn splice_old_not_found_is_clean_feedback_and_candidate_unchanged() {
    let d = mk_ws();
    let r1 = hs_loop::editapply::apply_blocks(d.path(), &[blk("app.py", "x = 1\n", "x = 2\n")]);
    assert_eq!(r1["applied"], true, "{r1}");
    let r2 = hs_loop::editapply::apply_blocks(
        d.path(),
        &[blk("lib.py", "    return 99\n", "    return 2\n")],
    );
    assert_eq!(r2["applied"], false, "{r2}");
    assert!(r2["error"].as_str().unwrap().contains("lib.py"), "{r2}");
    let cd = hs_loop::editapply::cumulative_diff(d.path());
    assert!(
        cd["cumulative_diff"].as_str().unwrap().contains("+x = 2"),
        "{cd}"
    );
    let _ = hs_loop::editapply::reset(d.path());
}

#[test]
fn splice_ambiguous_old_demands_more_context() {
    let d = mk_ws();
    let r = hs_loop::editapply::apply_blocks(d.path(), &[blk("dup.py", "a = 1\n", "a = 9\n")]);
    assert_eq!(r["applied"], false, "{r}");
    let e = r["error"].as_str().unwrap();
    assert!(e.contains('2') && e.contains("context"), "{e}");
    let _ = hs_loop::editapply::reset(d.path());
}

#[test]
fn splice_whitespace_fallback_applies_on_unique_fuzzy_match() {
    let d = mk_ws();
    // file has trailing spaces after y = 3; block omits them
    let r = hs_loop::editapply::apply_blocks(d.path(), &[blk("trail.py", "y = 3\n", "y = 4\n")]);
    assert_eq!(r["applied"], true, "{r}");
    let cd = r["cumulative_diff"].as_str().unwrap();
    assert!(cd.contains("+y = 4"), "{cd}");
    let _ = hs_loop::editapply::reset(d.path());
}

#[test]
fn splice_multi_file_one_call_all_or_nothing() {
    let d = mk_ws();
    let r = hs_loop::editapply::apply_blocks(
        d.path(),
        &[
            blk("app.py", "x = 1\n", "x = 2\n"),
            blk("lib.py", "    return 1\n", "    return 2\n"),
        ],
    );
    assert_eq!(r["applied"], true, "{r}");
    let cd = r["cumulative_diff"].as_str().unwrap();
    assert!(
        cd.contains("+x = 2") && cd.contains("+    return 2"),
        "{cd}"
    );
    // one bad block anywhere fails the whole call and writes nothing
    let d2 = mk_ws();
    let r2 = hs_loop::editapply::apply_blocks(
        d2.path(),
        &[
            blk("app.py", "x = 1\n", "x = 2\n"),
            blk("lib.py", "    return 99\n", "    return 2\n"),
        ],
    );
    assert_eq!(r2["applied"], false, "{r2}");
    assert_eq!(
        hs_loop::editapply::cumulative_diff(d2.path())["has_candidate"],
        true
    );
    assert!(
        !hs_loop::editapply::cumulative_diff(d2.path())["cumulative_diff"]
            .as_str()
            .unwrap()
            .contains("+x = 2"),
        "no partial writes"
    );
    let _ = hs_loop::editapply::reset(d.path());
    let _ = hs_loop::editapply::reset(d2.path());
}

#[test]
fn splice_path_escape_rejected() {
    let d = mk_ws();
    let r = hs_loop::editapply::apply_blocks(d.path(), &[blk("../evil.py", "x\n", "y\n")]);
    assert_eq!(r["applied"], false, "{r}");
    let _ = hs_loop::editapply::reset(d.path());
}

fn serve_roundtrip(
    bin: &str,
    envs: &[(&str, &std::path::Path)],
    reqs: &str,
) -> Vec<serde_json::Value> {
    let mut c = Command::new(bin);
    for (k, v) in envs {
        c.env(k, v);
    }
    let mut p = c
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let mut stdin = p.stdin.take().unwrap();
    let reqs = reqs.to_string();
    std::thread::spawn(move || {
        stdin.write_all(reqs.as_bytes()).unwrap();
        drop(stdin);
    });
    let out = p.wait_with_output().unwrap();
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect()
}

#[test]
fn plugin_applies_edits_blocks() {
    let d = mk_ws();
    let bin = env!("CARGO_BIN_EXE_hs-plugin-editapply");
    let envs = [("HS_SWE_WORKSPACE", d.path())];
    let r = serve_roundtrip(bin, &envs,
        "{\"id\":1,\"method\":\"tool.call\",\"params\":{\"args\":{\"edits\":[{\"path\":\"app.py\",\"old\":\"x = 1\\n\",\"new\":\"x = 2\\n\"}]}}}\n");
    assert_eq!(r[0]["result"]["applied"], true, "{r:?}");
    assert!(
        r[0]["result"]["cumulative_diff"]
            .as_str()
            .unwrap()
            .contains("+x = 2"),
        "{r:?}"
    );
    let _ = hs_loop::editapply::reset(d.path());
}

#[test]
fn plugin_retires_raw_diff_with_steering_error() {
    let d = mk_ws();
    let bin = env!("CARGO_BIN_EXE_hs-plugin-editapply");
    let envs = [("HS_SWE_WORKSPACE", d.path())];
    let r = serve_roundtrip(bin, &envs,
        "{\"id\":1,\"method\":\"tool.call\",\"params\":{\"args\":{\"diff\":\"diff --git a/app.py b/app.py\\n--- a/app.py\\n+++ b/app.py\\n@@ -1 +1 @@\\n-x = 1\\n+x = 2\\n\"}}}\n");
    let e = r[0]["error"].as_str().unwrap_or("");
    assert!(e.contains("edits"), "steering error names the new arg: {e}");
    assert!(!std::fs::read_to_string(d.path().join("app.py"))
        .unwrap()
        .contains("x = 2"));
    let _ = hs_loop::editapply::reset(d.path());
}
