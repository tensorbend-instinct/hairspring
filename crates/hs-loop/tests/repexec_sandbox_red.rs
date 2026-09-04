//! RED contract tests for the exec sandbox gate (Eric's ruling 2026-09-04:
//! open shell, any command, ZERO list - safety from isolation only, via the
//! on-box bwrap primitive). API: hs_loop::repexec::run_sandboxed(ws,
//! answer_path, command, timeout_secs) -> Value. The allowlist is deleted,
//! not kept.

use std::process::Command;

fn mk_ws() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    let ws = dir.path();
    std::fs::write(ws.join("app.py"), "x = 1\n").unwrap();
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

const PATCH_OK: &str = "```diff\ndiff --git a/app.py b/app.py\n--- a/app.py\n+++ b/app.py\n@@ -1 +1 @@\n-x = 1\n+x = 2\n```\n";

fn ws_with_answer() -> tempfile::TempDir {
    let d = mk_ws();
    std::fs::write(d.path().join("answer.txt"), PATCH_OK).unwrap();
    d
}

#[test]
fn sandbox_runs_any_command_open_shell() {
    let d = ws_with_answer();
    // a command no coding-allowlist would ever name: the point is it RUNS
    let r = hs_loop::repexec::run_sandboxed(d.path(), &d.path().join("answer.txt"),
        "cat app.py | rev | rev", 30);
    assert_eq!(r["applied"], true, "{r}");
    assert_eq!(r["exit_code"], 0, "{r}");
    assert!(r["stdout"].as_str().unwrap().contains("x = 2"), "{r}");
    // live ws untouched
    assert_eq!(std::fs::read_to_string(d.path().join("app.py")).unwrap(), "x = 1\n");
}

#[test]
fn sandbox_hides_host_filesystem_secrets_and_live_ws() {
    let d = ws_with_answer();
    let secret = d.path().join("secret.key");
    std::fs::write(&secret, "TOPSECRET").unwrap();
    // the host home, the mission run dir outside the worktree, and the live
    // ws path must not exist inside the sandbox
    let r = hs_loop::repexec::run_sandboxed(d.path(), &d.path().join("answer.txt"),
        "ls /home 2>&1; cat /home/sandbox/.keys/glm.key 2>&1; hostname", 30);
    let out = format!("{}{}", r["stdout"].as_str().unwrap(), r["stderr"].as_str().unwrap());
    assert!(!out.contains("TOPSECRET"), "{out}");
    assert!(!out.contains("sandbox"), "/home must not show the real user: {out}");
    // and nothing under the scratch leaks the host run dir
    let r2 = hs_loop::repexec::run_sandboxed(d.path(), &d.path().join("answer.txt"),
        &format!("cat {} 2>&1", secret.display()), 30);
    let out2 = format!("{}{}", r2["stdout"].as_str().unwrap(), r2["stderr"].as_str().unwrap());
    assert!(!out2.contains("TOPSECRET"), "host path must not resolve: {out2}");
}

#[test]
fn sandbox_network_is_off_even_for_localhost() {
    let d = ws_with_answer();
    // 127.0.0.1:8787 is the live relay on this box during missions; if the
    // child can reach it the net namespace is not empty
    let r = hs_loop::repexec::run_sandboxed(d.path(), &d.path().join("answer.txt"),
        "python3 -c \"import socket; socket.create_connection(('127.0.0.1',8787),timeout=3)\" 2>&1; echo EXIT=$?",
        30);
    let out = format!("{}{}", r["stdout"].as_str().unwrap(), r["stderr"].as_str().unwrap());
    assert!(out.contains("EXIT=1") || out.contains("Network is unreachable") || out.contains("Errno 101"),
        "network must be off by construction: {out}");
}

#[test]
fn sandbox_env_is_scrubbed() {
    let d = ws_with_answer();
    let r = hs_loop::repexec::run_sandboxed(d.path(), &d.path().join("answer.txt"), "env", 30);
    let out = r["stdout"].as_str().unwrap();
    assert!(!out.contains("HS_"), "no mission env may leak: {out}");
    assert!(!out.contains("KEY"), "no key material may leak: {out}");
}

#[test]
fn sandbox_kills_memory_bombs() {
    let d = ws_with_answer();
    let t0 = std::time::Instant::now();
    let r = hs_loop::repexec::run_sandboxed(d.path(), &d.path().join("answer.txt"),
        "tail /dev/zero", 30);
    assert!(t0.elapsed().as_secs() < 30, "rlimit must kill, not the caller's patience");
    assert!(r["exit_code"].as_i64().unwrap_or(0) != 0 || r["timed_out"] == true,
        "bomber must not exit 0: {r}");
}

#[test]
fn sandbox_timeout_and_cleanup_still_hold() {
    let d = ws_with_answer();
    let t0 = std::time::Instant::now();
    let r = hs_loop::repexec::run_sandboxed(d.path(), &d.path().join("answer.txt"), "sleep 30", 2);
    assert!(t0.elapsed().as_secs() < 15);
    assert_eq!(r["timed_out"], true, "{r}");
    let wt = Command::new("git").args(["worktree", "list", "--porcelain"])
        .current_dir(d.path()).output().unwrap();
    assert_eq!(String::from_utf8_lossy(&wt.stdout).matches("worktree ").count(), 1,
        "scratch removed after timeout");
}
