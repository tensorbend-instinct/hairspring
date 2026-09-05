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

/// Fix 1 (Eric, 2026-09-05): the mission sandbox is a full machine floor -
/// real shell, root, writable system roots, package managers, network ON.
/// The old toy sandbox (unshare-all, ro toolchain, no network) was the
/// harness lying to the model about what a mission can do.
#[test]
fn sandbox_network_is_on_for_missions() {
    let d = ws_with_answer();
    // serve a file INSIDE the sandbox and curl it from the same child:
    // loopback only works when the net namespace is real
    let r = hs_loop::repexec::run_sandboxed(d.path(), &d.path().join("answer.txt"),
        "cd /ws && (python3 -m http.server 8873 --bind 127.0.0.1 >/dev/null 2>&1 &) && sleep 1 && curl -s -o /dev/null -w '%{http_code}' http://127.0.0.1:8873/",
        30);
    let out = format!("{}{}", r["stdout"].as_str().unwrap(), r["stderr"].as_str().unwrap());
    assert!(out.contains("200"), "loopback http must work with network on: {out}");
}

#[test]
fn sandbox_is_a_full_machine_floor() {
    let d = ws_with_answer();
    let r = hs_loop::repexec::run_sandboxed(d.path(), &d.path().join("answer.txt"),
        "echo UID=$(id -u); touch /usr/local/.hs-floor-probe && rm /usr/local/.hs-floor-probe && echo USRLOCAL-RW; for t in python3 apt-get; do command -v $t >/dev/null 2>&1 && echo HAVE-$t; done; echo HOME=$HOME; echo PATH=$PATH",
        30);
    let out = format!("{}{}", r["stdout"].as_str().unwrap(), r["stderr"].as_str().unwrap());
    assert_eq!(r["exit_code"], 0, "{out}");
    assert!(out.contains("UID=0"), "missions run as root on the machine floor: {out}");
    assert!(out.contains("USRLOCAL-RW"), "system roots are writable: {out}");
    assert!(out.contains("HAVE-python3"), "python3 on the floor: {out}");
    assert!(out.contains("HAVE-apt-get"), "apt-get on the floor: {out}");
    assert!(out.contains("HOME=/root"), "root's home: {out}");
    assert!(out.contains("/root/.cargo/bin"), "cargo on PATH: {out}");
}

#[test]
fn sandbox_still_hides_host_secrets() {
    let d = ws_with_answer();
    // the floor is the whole machine EXCEPT host secret material: /home
    // (glm.key), /mnt (ledger/bundle), /root/.ssh, /root/.git-credentials
    let r = hs_loop::repexec::run_sandboxed(d.path(), &d.path().join("answer.txt"),
        "ls /home 2>&1; ls /mnt 2>&1; ls -a /root/.ssh 2>&1; wc -c /root/.git-credentials 2>&1",
        30);
    let out = format!("{}{}", r["stdout"].as_str().unwrap(), r["stderr"].as_str().unwrap());
    assert!(out.contains("No such file or directory"), "/home must not exist inside: {out}");
    assert!(!out.contains("instinct-nvme"), "/mnt must not exist inside: {out}");
    assert!(!out.contains("authorized_keys"), "ssh keys must not be readable: {out}");
    assert!(out.contains("/root/.git-credentials: 0"), "git-credentials masked to zero bytes: {out}");
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
