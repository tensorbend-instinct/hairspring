//! RED (2026-09-06): Codex-format edit.patch + computed answer.submit.
//!
//! The model never authors diff syntax: it edits the persistent candidate
//! with the Codex `apply_patch` grammar (vendored, hs-applypatch crate) and
//! answer.submit computes the final unified diff with git. This kills the
//! session's observed failure classes by construction: corrupt hand-written
//! hunks (8619 seq=51, 3314 seq=105), empty fenced submissions (8609
//! seq=121), and model-text-contaminated answer files.
use hs_loop::editapply;
use std::path::{Path, PathBuf};
use std::process::Command;

fn fixture_ws() -> PathBuf {
    // Uniqueness contract: clock nanos REPEAT across threads on this
    // fleet (measured dup values, see commit ef0312e); a pid+nanos dir
    // name collided here (concurrent `git init` on the same dir:
    // "cannot copy ... description: File exists"). pid + process-local
    // atomic counter is deterministic and can never collide cross-thread.
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
    let uniq = format!(
        "applypatch-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    );
    let ws = std::env::temp_dir().join(uniq);
    std::fs::create_dir_all(ws.join("src")).unwrap();
    let git = |args: &[&str]| {
        let o = Command::new("git")
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
    };
    git(&["init", "-q"]);
    git(&["config", "user.email", "t@t"]);
    git(&["config", "user.name", "t"]);
    std::fs::write(ws.join("src/main.py"), "def main():\n    return 1\n").unwrap();
    std::fs::write(ws.join("README.md"), "# demo\n").unwrap();
    git(&["add", "-A"]);
    git(&["commit", "-qm", "base"]);
    ws
}

fn edit_patch(ws: &Path, patch: &str) -> serde_json::Value {
    editapply::apply_codex_patch(ws, patch)
}

#[test]
fn edit_patch_update_applies_to_candidate_and_returns_cumulative_diff() {
    let ws = fixture_ws();
    let v = edit_patch(&ws, "*** Begin Patch\n*** Update File: src/main.py\n@@\n-    return 1\n+    return 2\n*** End Patch\n");
    assert!(v.get("$error").is_none(), "unexpected error: {v}");
    let diff = v["cumulative_diff"].as_str().expect("cumulative_diff");
    assert!(diff.contains("+    return 2"), "diff: {diff}");
    // candidate holds the edit; the LIVE ws is untouched
    let cand_file = std::fs::read_to_string(ws.join("src/main.py")).unwrap();
    assert!(
        cand_file.contains("return 1"),
        "live ws must never be touched"
    );
}

#[test]
fn edit_patch_add_and_delete_file() {
    let ws = fixture_ws();
    let v = edit_patch(&ws, "*** Begin Patch\n*** Add File: src/new.py\n+print('new')\n*** Delete File: README.md\n*** End Patch\n");
    assert!(v.get("$error").is_none(), "unexpected error: {v}");
    let diff = v["cumulative_diff"].as_str().unwrap();
    assert!(diff.contains("new file mode"), "add in diff: {diff}");
    assert!(diff.contains("deleted file mode"), "delete in diff: {diff}");
    assert!(diff.contains("+print('new')"), "diff: {diff}");
}

#[test]
fn edit_patch_wrong_context_named_error_candidate_untouched() {
    // replay class: 8619 seq=51 / 3314 seq=105 - model's context does not
    // match the file. Old path: corrupt hand-written hunk burned a checker
    // cycle. New path: named $error, candidate byte-identical, no burn.
    let ws = fixture_ws();
    let before = editapply::cumulative_diff(&ws)["cumulative_diff"]
        .as_str()
        .unwrap_or("")
        .to_string();
    let v = edit_patch(&ws, "*** Begin Patch\n*** Update File: src/main.py\n@@\n-    return NINETY_NINE\n+    return 2\n*** End Patch\n");
    let err = v["$error"].as_str().expect("must error on missing context");
    assert!(err.contains("src/main.py"), "error names the file: {err}");
    let after = editapply::cumulative_diff(&ws)["cumulative_diff"]
        .as_str()
        .unwrap_or("")
        .to_string();
    assert_eq!(before, after, "candidate diff must be untouched on failure");
}

#[test]
fn edit_patch_rejects_escaping_paths() {
    let ws = fixture_ws();
    for bad in ["../evil.txt", "/etc/passwd"] {
        let v = edit_patch(
            &ws,
            &format!("*** Begin Patch\n*** Add File: {bad}\n+x\n*** End Patch\n"),
        );
        assert!(
            v["$error"].as_str().unwrap_or("").contains("path"),
            "escape must be rejected: {bad} -> {v}"
        );
    }
}

#[test]
fn answer_submit_writes_git_apply_clean_diff_including_new_files() {
    let ws = fixture_ws();
    let _ = edit_patch(&ws, "*** Begin Patch\n*** Update File: src/main.py\n@@\n-    return 1\n+    return 2\n*** Add File: src/added.py\n+x = 1\n*** End Patch\n");
    let answer = ws.join("answer.diff");
    let v = editapply::answer_submit(&ws, &answer);
    assert!(v.get("$error").is_none(), "submit: {v}");
    let text = std::fs::read_to_string(&answer).unwrap();
    assert!(
        text.starts_with("diff --git"),
        "pure git diff, no fence/prose: {:?}",
        &text[..80.min(text.len())]
    );
    assert!(!text.contains("```"), "never any markdown in the answer");
    assert!(text.contains("+    return 2") && text.contains("+x = 1"));
    // property: computed output ALWAYS applies clean to a pristine clone
    let pristine = std::env::temp_dir().join(format!(
        "{}-pristine",
        ws.file_name().unwrap().to_string_lossy()
    ));
    let o = Command::new("git")
        .args(["clone", "-q"])
        .arg(&ws)
        .arg(&pristine)
        .output()
        .unwrap();
    assert!(o.status.success());
    let chk = Command::new("git")
        .args(["apply", "--check"])
        .arg(&answer)
        .current_dir(&pristine)
        .output()
        .unwrap();
    assert!(
        chk.status.success(),
        "submit output must git-apply clean: {}",
        String::from_utf8_lossy(&chk.stderr)
    );
}

#[test]
fn answer_submit_empty_candidate_named_error_no_file_written() {
    // replay class: 8609 seq=121 - model submitted a literal empty fence
    // (```diff\n```) as its answer. New path makes that impossible: no
    // candidate edits -> steering error, nothing written.
    let ws = fixture_ws();
    let answer = ws.join("answer.diff");
    let v = editapply::answer_submit(&ws, &answer);
    let err = v["$error"].as_str().expect("must error on empty candidate");
    assert!(err.contains("edit.patch"), "steers to the edit tool: {err}");
    assert!(!answer.exists(), "no answer file may be written");
}
