//! Long tool output keeps its tail inline AND is written in full to a file
//! outside the repo whose path is returned; short output is untouched.
fn ws() -> tempfile::TempDir {
    let d = tempfile::tempdir().unwrap();
    let g = |a: &[&str]| {
        std::process::Command::new("git").args(a).current_dir(d.path())
            .env("GIT_AUTHOR_NAME", "t").env("GIT_AUTHOR_EMAIL", "t@t")
            .env("GIT_COMMITTER_NAME", "t").env("GIT_COMMITTER_EMAIL", "t@t")
            .output().unwrap()
    };
    std::fs::write(d.path().join("a.txt"), "x\n").unwrap();
    g(&["init", "-q"]); g(&["add", "."]); g(&["commit", "-qm", "i"]);
    d
}

const PATCH: &str = "```diff\ndiff --git a/a.txt b/a.txt\n--- a/a.txt\n+++ b/a.txt\n@@ -1 +1 @@\n-x\n+y\n```\n";

#[test]
fn long_output_spills_full_text_and_returns_path() {
    let d = ws();
    let ans = d.path().join("answer.txt");
    std::fs::write(&ans, PATCH).unwrap();
    let v = hs_loop::repexec::run_host(d.path(), &ans, "seq 1 6000", 20);
    let p = v["stdout_full_path"].as_str().unwrap_or_else(|| panic!("no path: {v}"));
    assert!(!p.starts_with(d.path().to_str().unwrap()), "spill must be outside the repo: {p}");
    let full = std::fs::read_to_string(p).unwrap();
    assert!(full.starts_with("1\n") && full.trim_end().ends_with("6000"));
    assert_eq!(v["stdout_total_bytes"].as_u64().unwrap() as usize, full.len());
    assert!(v["stdout"].as_str().unwrap().trim_end().ends_with("6000"), "tail stays inline");
    assert!(v["stdout"].as_str().unwrap().len() < full.len());
    let _ = std::fs::remove_file(p);
}

#[test]
fn short_output_has_no_spill_fields() {
    let d = ws();
    let ans = d.path().join("answer.txt");
    std::fs::write(&ans, PATCH).unwrap();
    let v = hs_loop::repexec::run_host(d.path(), &ans, "echo hi", 20);
    assert!(v.get("stdout_full_path").is_none(), "{v}");
    assert_eq!(v["stdout"].as_str().unwrap().trim(), "hi");
}
