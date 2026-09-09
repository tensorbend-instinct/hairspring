//! Hostile-pass flake hunt (suite-m46): under full-suite parallel load,
//! `exec_preflight` saw `run_sandboxed` return a result with NO "applied"
//! key (a $error machinery arm) while passing 3/3 in isolation. The
//! honest move is to reproduce the transient arm and read its exact
//! message, not explain it away. RED: no machinery arm may fire under
//! concurrent load - transient spawn failure is retried, never silent.

const PATCH_OK: &str = "```diff\ndiff --git a/app.py b/app.py\nindex 9daeafb..b8626c8 100644\n--- a/app.py\n+++ b/app.py\n@@ -1 +1 @@\n-x = 1\n+x = 2\n```\n";

fn mk_ws() -> tempfile::TempDir {
    let d = tempfile::tempdir().unwrap();
    std::fs::write(d.path().join("app.py"), "x = 1\n").unwrap();
    let out = std::process::Command::new("git")
        .args(["init", "-q", "."])
        .current_dir(d.path())
        .output()
        .unwrap();
    assert!(out.status.success(), "git init: {out:?}");
    for args in [
        vec!["config", "user.email", "t@t"],
        vec!["config", "user.name", "t"],
        vec!["add", "."],
        vec!["commit", "-q", "-m", "init"],
    ] {
        let out = std::process::Command::new("git")
            .args(&args)
            .current_dir(d.path())
            .output()
            .unwrap();
        assert!(out.status.success(), "git {args:?}: {out:?}");
    }
    d
}

#[test]
fn stress_run_sandboxed_never_hits_a_machinery_arm_under_load() {
    const THREADS: usize = 8;
    const ITERS: usize = 25;
    let mut handles = Vec::new();
    for tid in 0..THREADS {
        handles.push(std::thread::spawn(move || {
            let ws = mk_ws();
            let ans = ws.path().join("answer.txt");
            std::fs::write(&ans, PATCH_OK).unwrap();
            for i in 0..ITERS {
                let r = hs_loop::repexec::run_sandboxed(ws.path(), &ans, "true", 30);
                assert!(
                    r.get("applied").is_some(),
                    "thread {tid} iter {i}: machinery arm fired, exact result: {r}"
                );
                assert_eq!(r["applied"], true, "thread {tid} iter {i}: {r}");
            }
        }));
    }
    for h in handles {
        h.join().unwrap();
    }
}

/// RED pin for the suite-m46 flake class: a TRANSIENT machinery failure
/// (spawn EAGAIN, one-shot worktree error under host-wide load) must be
/// retried, never surfaced as a silent $error result. The fault-injection
/// hook makes the first two git spawns in prep fail with `WouldBlock`;
/// pre-fix that result was the one-shot $error arm (`r["applied"]` Null).
#[test]
fn transient_git_spawn_failure_is_retried_not_surfaced() {
    // FIXME: Audit that the environment access only happens in single-threaded code.
    unsafe { std::env::set_var("HS_REPEXEC_TEST_FAIL_GIT_SPAWNS", "2") };
    let d = tempfile::tempdir().unwrap();
    std::fs::write(d.path().join("app.py"), "x = 1\n").unwrap();
    for args in [
        vec!["init", "-q", "."],
        vec!["config", "user.email", "t@t"],
        vec!["config", "user.name", "t"],
        vec!["add", "."],
        vec!["commit", "-q", "-m", "init"],
    ] {
        let out = std::process::Command::new("git")
            .args(&args)
            .current_dir(d.path())
            .output()
            .unwrap();
        assert!(out.status.success(), "git {args:?}: {out:?}");
    }
    let ans = d.path().join("answer.txt");
    std::fs::write(&ans, PATCH_OK).unwrap();
    let r = hs_loop::repexec::run_sandboxed(d.path(), &ans, "true", 30);
    assert_eq!(r["applied"], true, "transient spawn failure recovered via retry: {r}");
}
