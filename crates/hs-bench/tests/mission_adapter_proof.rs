//! GATE 8 BENCHMARK PREP 2 - mission adapter, offline (no paid model calls).
//!
//! A BenchInstance becomes a real workspace: clone the repo, check out the
//! base commit, apply the model's patch, run FAIL_TO_PASS/PASS_TO_PASS.
//! Result taxonomy matches SWE-bench: resolved / unresolved / no_apply
//! (patch did not apply is NOT a test failure) / budget_killed.
//!
//! Falsifiable: if a patch that git cannot apply is reported as a test
//! failure, the taxonomy is broken. If a gold patch on a real fixture repo
//! does not flip its FAIL_TO_PASS test, the plumbing is broken.

use hs_bench::*;
use std::path::Path;
use std::process::Command;

/// Build a tiny local git repo: code.txt "broken" + tests/test_fix.sh that
/// passes only when code.txt contains "fixed". Returns (repo_path, base_commit).
fn make_fixture_repo(dir: &Path) -> (PathBuf2, String) {
    let repo = dir.join("repo");
    std::fs::create_dir_all(repo.join("tests")).unwrap();
    std::fs::write(repo.join("code.txt"), "broken\n").unwrap();
    std::fs::write(
        repo.join("tests/test_fix.sh"),
        "#!/bin/sh\ngrep -q '^fixed$' code.txt\n",
    )
    .unwrap();
    let git = |args: &[&str]| {
        let st = Command::new("git")
            .args(args)
            .current_dir(&repo)
            .status()
            .unwrap();
        assert!(st.success(), "git {args:?} failed");
    };
    git(&["init", "-q"]);
    git(&["config", "user.email", "bench@fixture"]);
    git(&["config", "user.name", "bench"]);
    git(&["add", "-A"]);
    git(&["commit", "-qm", "base"]);
    let out = Command::new("git")
        .args(["rev-parse", "HEAD"])
        .current_dir(&repo)
        .output()
        .unwrap();
    (PathBuf2(repo), String::from_utf8(out.stdout).unwrap().trim().to_string())
}

struct PathBuf2(std::path::PathBuf);
impl std::fmt::Display for PathBuf2 {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0.display())
    }
}

fn instance(repo: &Path, commit: &str) -> BenchInstance {
    BenchInstance {
        instance_id: "fixture__git-1".into(),
        repo: repo.to_string_lossy().to_string(),
        base_commit: commit.into(),
        problem_statement: "code.txt must contain the word fixed".into(),
        patch: "--- a/code.txt\n+++ b/code.txt\n@@ -1 +1 @@\n-broken\n+fixed\n".into(),
        fail_to_pass: vec!["tests/test_fix.sh".into()],
        pass_to_pass: vec![],
    }
}

#[test]
fn prep_workspace_clones_and_checks_out_base_commit() {
    let tmp = tempfile::tempdir().unwrap();
    let (repo, base) = make_fixture_repo(tmp.path());
    let inst = instance(&repo.0, &base);
    let cache = tmp.path().join("cache");
    let ws = prep_workspace(&inst, &cache).unwrap();
    assert!(ws.join("code.txt").exists());
    assert!(ws.join("problem_statement.md").exists());
    assert!(ws.join("tests/test_fix.sh").exists());
    assert_eq!(
        std::fs::read_to_string(ws.join("code.txt")).unwrap(),
        "broken\n",
        "workspace must be at base commit state"
    );
}

#[test]
fn gold_patch_applies_and_flips_fail_to_pass() {
    let tmp = tempfile::tempdir().unwrap();
    let (repo, base) = make_fixture_repo(tmp.path());
    let inst = instance(&repo.0, &base);
    let ws = prep_workspace(&inst, &tmp.path().join("cache")).unwrap();

    // before patch: FAIL_TO_PASS fails
    let pre = run_tests(&ws, &inst.fail_to_pass, &inst.pass_to_pass).unwrap();
    assert!(!pre.all_passing(), "test must fail at base commit");

    let applied = apply_model_patch(&ws, &inst.patch).unwrap();
    assert!(matches!(applied, ApplyResult::Applied), "gold patch must apply");

    let post = run_tests(&ws, &inst.fail_to_pass, &inst.pass_to_pass).unwrap();
    assert!(post.all_passing(), "FAIL_TO_PASS must pass after gold patch");
}

#[test]
fn malformed_patch_is_no_apply_not_a_test_failure() {
    let tmp = tempfile::tempdir().unwrap();
    let (repo, base) = make_fixture_repo(tmp.path());
    let inst = instance(&repo.0, &base);
    let ws = prep_workspace(&inst, &tmp.path().join("cache")).unwrap();
    let r = apply_model_patch(&ws, "this is not a diff at all").unwrap();
    assert!(
        matches!(r, ApplyResult::NoApply(_)),
        "malformed patch must be NoApply, got {r:?}"
    );
}

#[test]
fn empty_patch_leaves_tests_failing() {
    let tmp = tempfile::tempdir().unwrap();
    let (repo, base) = make_fixture_repo(tmp.path());
    let inst = instance(&repo.0, &base);
    let ws = prep_workspace(&inst, &tmp.path().join("cache")).unwrap();
    let r = apply_model_patch(&ws, "").unwrap();
    assert!(matches!(r, ApplyResult::NoApply(_)) || {
        let post = run_tests(&ws, &inst.fail_to_pass, &inst.pass_to_pass).unwrap();
        !post.all_passing()
    });
}
