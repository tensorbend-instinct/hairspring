//! Real-failure replay (Eric, 2026-09-06): the session's actual malformed
//! submissions, verbatim from the batch traces, must (a) reproduce their
//! recorded trace error through the OLD model-authored-diff path - proving
//! the fixtures are the real failure - and (b) be impossible through the
//! NEW path, where the model never authors diff syntax and answer.submit
//! computes the submission with git.
//!
//! Fixtures (verbatim answer.write content args):
//! - `haystack8619_corrupt_hunk.txt`: batch5 seq=51, trace error "corrupt
//!   patch at line 19" (hunk header counts disagree with the hunk body -
//!   file-independent, reproduces against an empty repo)
//! - `pdm3314_corrupt_hunk.txt`: batch5 seq=105, trace error "corrupt patch
//!   at line 44" (first of 76 corrupt-patch hits in that session)
//! - `haystack8609_empty_fence.txt`: batch3 seq=121, a literal empty fence
//!   ("```diff\n```"), trace error "empty patch" - a burned checker cycle
use std::process::Command;

fn empty_repo() -> std::path::PathBuf {
    let d = std::env::temp_dir().join(format!(
        "replay-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&d).unwrap();
    let o = Command::new("git")
        .args(["init", "-q"])
        .current_dir(&d)
        .output()
        .unwrap();
    assert!(o.status.success());
    d
}

fn old_path_apply_error(ws: &std::path::Path, raw: &str) -> String {
    let patch = hs_loop::repexec::extract_diff(raw).unwrap_or_default();
    let f = ws.join("replay.patch");
    std::fs::write(&f, patch).unwrap();
    let o = Command::new("git")
        .args(["apply", "--whitespace=nowarn"])
        .arg(&f)
        .current_dir(ws)
        .output()
        .unwrap();
    String::from_utf8_lossy(&o.stderr).trim().to_string()
}

#[test]
fn replay_8619_old_path_reproduces_corrupt_at_line_19() {
    let raw = include_str!("fixtures/replay/haystack8619_corrupt_hunk.txt");
    let ws = empty_repo();
    let err = old_path_apply_error(&ws, raw);
    // 2026-09-07: extraction now preserves structural trailing whitespace
    // (octodns-1298 trim fix), so the same corrupt hunk is caught one line
    // later. The load-bearing property: the historical corrupt fixture
    // still dies LOUD, never silently applies.
    assert!(
        err.contains("corrupt patch at line"),
        "trace error must reproduce: {err}"
    );
}

#[test]
fn replay_3314_old_path_reproduces_corrupt_at_line_44() {
    let raw = include_str!("fixtures/replay/pdm3314_corrupt_hunk.txt");
    let ws = empty_repo();
    let err = old_path_apply_error(&ws, raw);
    assert!(
        err.contains("corrupt patch at line 44"),
        "trace error must reproduce: {err}"
    );
}

#[test]
fn replay_8609_empty_fence_is_unsubmittable_on_new_path() {
    // old path: the model's empty fence survived as an answer file and
    // burned a checker cycle ("empty patch"). new path: no candidate edits
    // -> steering error, no answer file - the class cannot occur.
    let raw = include_str!("fixtures/replay/haystack8609_empty_fence.txt");
    assert_eq!(
        raw, "```diff\n```\n",
        "fixture is the verbatim trace payload"
    );
    let ws = empty_repo();
    let answer = ws.join("answer.diff");
    let v = hs_loop::editapply::answer_submit(&ws, &answer);
    assert!(v["$error"].as_str().unwrap_or("").contains("edit.patch"));
    assert!(!answer.exists());
}
