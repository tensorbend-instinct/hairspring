//! repo.read + repo.search tool cores: sandboxed read-only repo access.
//! Red-first: contract pinned before implementation.
use std::fs;

fn ws() -> tempfile::TempDir {
    let d = tempfile::tempdir().unwrap();
    fs::create_dir_all(d.path().join("src/pkg")).unwrap();
    fs::write(d.path().join("src/pkg/mod.rs"), "fn answer() -> u32 { 42 }\nfn helper() {}\n").unwrap();
    fs::write(d.path().join("README.md"), "readme body\nanswer here too\n").unwrap();
    fs::write(d.path().join("big.txt"), "x".repeat(200_000)).unwrap();
    d
}

#[test]
fn reads_a_file_inside_the_workspace() {
    let d = ws();
    let v = hs_loop::repotools::read_repo_file(d.path(), "src/pkg/mod.rs").unwrap();
    assert!(v["content"].as_str().unwrap().contains("fn answer"));
    assert_eq!(v["truncated"].as_bool().unwrap(), false);
}

#[test]
fn rejects_dotdot_escape() {
    let d = ws();
    let e = hs_loop::repotools::read_repo_file(d.path(), "../outside.txt").unwrap_err();
    assert!(e.contains("escape"), "error names the escape: {e}");
}

#[test]
fn rejects_absolute_path() {
    let d = ws();
    let e = hs_loop::repotools::read_repo_file(d.path(), "/etc/passwd").unwrap_err();
    assert!(e.contains("escape") || e.contains("absolute"), "error: {e}");
}

#[test]
fn symlink_escape_is_rejected() {
    let d = ws();
    std::os::unix::fs::symlink("/etc/hostname", d.path().join("link")).unwrap();
    let e = hs_loop::repotools::read_repo_file(d.path(), "link").unwrap_err();
    assert!(e.contains("escape"), "symlink out of workspace rejected: {e}");
}

#[test]
fn missing_file_is_a_clean_error() {
    let d = ws();
    let e = hs_loop::repotools::read_repo_file(d.path(), "src/nope.rs").unwrap_err();
    assert!(e.contains("not found") || e.contains("no such"), "error: {e}");
}

#[test]
fn large_files_are_capped_and_marked() {
    let d = ws();
    let v = hs_loop::repotools::read_repo_file(d.path(), "big.txt").unwrap();
    assert!(v["content"].as_str().unwrap().len() <= 41_000);
    assert_eq!(v["truncated"].as_bool().unwrap(), true);
    assert_eq!(v["total_bytes"].as_u64().unwrap(), 200_000);
}

#[test]
fn search_finds_matches_with_locations() {
    let d = ws();
    let v = hs_loop::repotools::search_repo(d.path(), "answer").unwrap();
    let m = v["matches"].as_array().unwrap();
    assert!(m.iter().any(|x| x["path"] == "src/pkg/mod.rs" && x["line"] == 1));
    assert!(m.iter().any(|x| x["path"] == "README.md" && x["line"] == 2));
}

#[test]
fn search_no_matches_is_empty_not_error() {
    let d = ws();
    let v = hs_loop::repotools::search_repo(d.path(), "zzz-absent").unwrap();
    assert_eq!(v["matches"].as_array().unwrap().len(), 0);
}

#[test]
fn search_skips_git_dir_and_caps_results() {
    let d = ws();
    fs::create_dir_all(d.path().join(".git")).unwrap();
    fs::write(d.path().join(".git/packed-refs"), "answer answer answer\n").unwrap();
    for i in 0..300 {
        fs::write(d.path().join(format!("f{i}.txt")), "answer\n").unwrap();
    }
    let v = hs_loop::repotools::search_repo(d.path(), "answer").unwrap();
    let m = v["matches"].as_array().unwrap();
    assert!(m.iter().all(|x| !x["path"].as_str().unwrap().starts_with(".git")));
    assert!(m.len() <= 100, "capped at 100, got {}", m.len());
    assert_eq!(v["capped"].as_bool().unwrap(), true);
}

#[test]
fn search_rejects_empty_pattern() {
    let d = ws();
    assert!(hs_loop::repotools::search_repo(d.path(), "").is_err());
}

// Ranged reads (red, 2026-09-04): jsinterp.py's target code sits at byte
// 36071 - a 20KB transcript entry cap made it invisible and the model
// re-read the same file 17 times in one arm. Paging is the faithful shape.
#[test]
fn windowed_read_returns_requested_lines() {
    let d = tempfile::tempdir().unwrap();
    let body: String = (1..=200).map(|i| format!("line {i}\n")).collect();
    std::fs::write(d.path().join("big.txt"), body).unwrap();
    let v = hs_loop::repotools::read_repo_window(d.path(), "big.txt", Some(50), Some(10))
        .unwrap();
    assert_eq!(v["start_line"], 50);
    assert_eq!(v["end_line"], 59);
    assert_eq!(v["total_lines"], 200);
    assert_eq!(v["truncated"], true, "more content beyond the window");
    let c = v["content"].as_str().unwrap();
    assert!(c.starts_with("line 50\n") && c.ends_with("line 59\n"), "got: {c}");
}

#[test]
fn windowed_read_past_end_is_empty_not_an_error() {
    let d = tempfile::tempdir().unwrap();
    std::fs::write(d.path().join("small.txt"), "a\nb\n").unwrap();
    let v = hs_loop::repotools::read_repo_window(d.path(), "small.txt", Some(500), Some(10))
        .unwrap();
    assert_eq!(v["content"], "");
    assert_eq!(v["total_lines"], 2);
    assert_eq!(v["truncated"], false);
}

#[test]
fn windowed_read_last_page_is_not_truncated() {
    let d = tempfile::tempdir().unwrap();
    let body: String = (1..=200).map(|i| format!("line {i}\n")).collect();
    std::fs::write(d.path().join("big.txt"), body).unwrap();
    let v = hs_loop::repotools::read_repo_window(d.path(), "big.txt", Some(195), Some(50))
        .unwrap();
    assert_eq!(v["end_line"], 200);
    assert_eq!(v["truncated"], false, "no content beyond the file end");
}
