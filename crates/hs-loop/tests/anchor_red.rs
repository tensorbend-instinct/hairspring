//! RED (2026-09-06, bake-off): anchor edit path (Grok hashline flavor) as a
//! hairspring-native flow: repo.read shows LINE:HASH, edit.anchor applies
//! validated ops to the candidate, answer.submit computes the diff - same
//! submit invariant as edit.patch.
use hs_loop::editapply;
use serde_json::json;
use std::path::PathBuf;
use std::process::Command;

fn fixture_ws() -> PathBuf {
    let uniq = format!(
        "anchor-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
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
    git(&["add", "-A"]);
    git(&["commit", "-qm", "base"]);
    ws
}

#[test]
fn anchored_read_renders_line_hash_prefixes() {
    let text = editapply::anchored_read("def main():\n    return 1\n");
    assert!(
        text.contains("1:") && text.contains("2:"),
        "line numbers: {text:?}"
    );
    assert!(text.contains('\u{2192}'), "arrow separator: {text:?}");
    assert!(text.contains("def main():"), "content: {text:?}");
}

#[test]
fn anchor_replace_applies_and_returns_fresh_anchors_and_cumulative() {
    let ws = fixture_ws();
    let shown = editapply::anchored_read("def main():\n    return 1\n");
    // anchor for line 2 ("    return 1"), quoted back exactly as rendered
    let line2 = shown.lines().nth(1).unwrap();
    let anchor = line2.split('\u{2192}').next().unwrap().to_string();
    let v = editapply::apply_anchor_edits(
        &ws,
        "src/main.py",
        json!([
            {"op": "replace", "anchor": anchor, "content": "    return 2"}
        ]),
    );
    assert!(v.get("$error").is_none(), "apply: {v}");
    assert!(
        v["snippet"].as_str().unwrap_or("").contains("return 2"),
        "fresh anchors back: {v}"
    );
    let diff = v["cumulative_diff"].as_str().unwrap_or("");
    assert!(diff.contains("+    return 2"), "cumulative: {diff}");
    let live = std::fs::read_to_string(ws.join("src/main.py")).unwrap();
    assert!(live.contains("return 1"), "live ws untouched");
}

#[test]
fn anchor_stale_or_wrong_named_error_candidate_untouched() {
    let ws = fixture_ws();
    let _ = editapply::apply_anchor_edits(
        &ws,
        "src/main.py",
        json!([
            {"op": "replace", "anchor": "2:zzzz", "content": "x"}
        ]),
    );
    let before = editapply::cumulative_diff(&ws)["cumulative_diff"]
        .as_str()
        .unwrap_or("")
        .to_string();
    let v = editapply::apply_anchor_edits(
        &ws,
        "src/main.py",
        json!([
            {"op": "replace", "anchor": "2:zzz:zzz", "content": "    return 9"}
        ]),
    );
    assert!(v.get("$error").is_some(), "bad anchor must error: {v}");
    let after = editapply::cumulative_diff(&ws)["cumulative_diff"]
        .as_str()
        .unwrap_or("")
        .to_string();
    assert_eq!(before, after, "candidate untouched on anchor failure");
}

#[test]
fn anchor_insert_after_and_write_ops() {
    let ws = fixture_ws();
    let v = editapply::apply_anchor_edits(
        &ws,
        "src/main.py",
        json!([
            {"op": "insert_after", "anchor": "EOF", "content": "    # done"}
        ]),
    );
    assert!(v.get("$error").is_none(), "insert_after EOF: {v}");
    assert!(
        v["cumulative_diff"]
            .as_str()
            .unwrap()
            .contains("+    # done"),
        "{v}"
    );
    let v2 = editapply::apply_anchor_edits(
        &ws,
        "src/new.py",
        json!([
            {"op": "write", "content": "x = 1\n"}
        ]),
    );
    assert!(v2.get("$error").is_none(), "write new file: {v2}");
    assert!(
        v2["cumulative_diff"].as_str().unwrap().contains("+x = 1"),
        "{v2}"
    );
}

#[test]
fn anchor_rejects_escaping_paths() {
    let ws = fixture_ws();
    let v = editapply::apply_anchor_edits(
        &ws,
        "../evil.txt",
        json!([
            {"op": "write", "content": "x"}
        ]),
    );
    assert!(v["$error"].as_str().unwrap_or("").contains("path"), "{v}");
}

#[test]
fn anchor_edits_then_submit_git_apply_clean() {
    let ws = fixture_ws();
    let shown = editapply::anchored_read("def main():\n    return 1\n");
    let line2 = shown.lines().nth(1).unwrap();
    let anchor = line2.split('\u{2192}').next().unwrap().to_string();
    let _ = editapply::apply_anchor_edits(
        &ws,
        "src/main.py",
        json!([
            {"op": "replace", "anchor": anchor, "content": "    return 2"}
        ]),
    );
    let answer = ws.join("answer.diff");
    let v = editapply::answer_submit(&ws, &answer);
    assert!(v.get("$error").is_none(), "submit: {v}");
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
        "anchor-path submit must git-apply clean: {}",
        String::from_utf8_lossy(&chk.stderr)
    );
}
