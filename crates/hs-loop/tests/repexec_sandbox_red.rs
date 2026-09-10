//! RED contract tests for the exec sandbox gate (Eric's ruling via iMessage
//! 2026-09-10: NO command ACL/whitelist - the mission may run any command it
//! needs; the hard boundary is the FILESYSTEM: no writes or deletes outside
//! the project workspace). API: `hs_loop::repexec::run_sandboxed(ws`,
//! `answer_path`, command, `timeout_secs`) -> Value. Isolation via the on-box
//! bwrap primitive: system roots read-only, /ws + /tmp the writable floor.

use std::process::Command;

fn mk_ws() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    let ws = dir.path();
    std::fs::write(ws.join("app.py"), "x = 1\n").unwrap();
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
    let r = hs_loop::repexec::run_sandboxed(
        d.path(),
        &d.path().join("answer.txt"),
        "cat app.py | rev | rev",
        30,
    );
    assert_eq!(r["applied"], true, "{r}");
    assert_eq!(r["exit_code"], 0, "{r}");
    assert!(r["stdout"].as_str().unwrap().contains("x = 2"), "{r}");
    // live ws untouched
    assert_eq!(
        std::fs::read_to_string(d.path().join("app.py")).unwrap(),
        "x = 1\n"
    );
}

#[test]
fn sandbox_hides_host_filesystem_secrets_and_live_ws() {
    let d = ws_with_answer();
    let secret = d.path().join("secret.key");
    std::fs::write(&secret, "TOPSECRET").unwrap();
    // the host home, the mission run dir outside the worktree, and the live
    // ws path must not exist inside the sandbox
    let r = hs_loop::repexec::run_sandboxed(d.path(), &d.path().join("answer.txt"),
        // NOTE: asserts are capture-safe (markers, not substring bans) - the
        // pre-fix redirect swallowed all but the last command's output
        "ls /home 2>/dev/null; echo LS-EXIT=$?; test -r /home/sandbox/.keys/glm.key && echo KEY-READABLE || echo KEY-ABSENT; hostname", 30);
    let out = format!(
        "{}{}",
        r["stdout"].as_str().unwrap(),
        r["stderr"].as_str().unwrap()
    );
    assert!(!out.contains("TOPSECRET"), "{out}");
    assert!(
        !out.contains("LS-EXIT=0"),
        "/home must not list inside: {out}"
    );
    assert!(
        out.contains("KEY-ABSENT"),
        "glm.key must not be readable: {out}"
    );
    assert!(!out.contains("KEY-READABLE"), "{out}");
    // and nothing under the scratch leaks the host run dir
    let r2 = hs_loop::repexec::run_sandboxed(
        d.path(),
        &d.path().join("answer.txt"),
        &format!("cat {} 2>&1", secret.display()),
        30,
    );
    let out2 = format!(
        "{}{}",
        r2["stdout"].as_str().unwrap(),
        r2["stderr"].as_str().unwrap()
    );
    assert!(
        !out2.contains("TOPSECRET"),
        "host path must not resolve: {out2}"
    );
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
        "cd /ws && (python3 -m http.server 8873 --bind 127.0.0.1 >/dev/null 2>&1 &) && for i in $(seq 1 100); do curl -s -o /dev/null http://127.0.0.1:8873/ && break; sleep 0.1; done && curl -s -o /dev/null -w '%{http_code}' http://127.0.0.1:8873/",
        30);
    let out = format!(
        "{}{}",
        r["stdout"].as_str().unwrap(),
        r["stderr"].as_str().unwrap()
    );
    assert!(
        out.contains("200"),
        "loopback http must work with network on: {out}"
    );
}

#[test]
fn sandbox_is_a_readable_machine_with_a_writable_workspace() {
    // Eric 2026-09-10 (iMessage): any command, but the filesystem outside
    // the project workspace is read-only. Tools still RESOLVE (read wide),
    // the mission still runs as root, cargo stays on PATH.
    let d = ws_with_answer();
    let r = hs_loop::repexec::run_sandboxed(d.path(), &d.path().join("answer.txt"),
        "echo UID=$(id -u); touch /usr/local/.hs-floor-probe 2>/dev/null && echo USRLOCAL-RW || echo USRLOCAL-RO; for t in python3 apt-get; do command -v $t >/dev/null 2>&1 && echo HAVE-$t; done; echo HOME=$HOME; echo PATH=$PATH",
        30);
    let out = format!(
        "{}{}",
        r["stdout"].as_str().unwrap(),
        r["stderr"].as_str().unwrap()
    );
    assert_eq!(r["exit_code"], 0, "{out}");
    assert!(
        out.contains("UID=0"),
        "missions run as root in the sandbox: {out}"
    );
    assert!(
        out.contains("USRLOCAL-RO"),
        "system roots are READ-ONLY (Eric 2026-09-10): {out}"
    );
    assert!(out.contains("HAVE-python3"), "python3 resolves: {out}");
    assert!(out.contains("HAVE-apt-get"), "apt-get resolves: {out}");
    assert!(out.contains("HOME=/root"), "root's home: {out}");
    assert!(out.contains("/root/.cargo/bin"), "cargo on PATH: {out}");
}

#[test]
fn sandbox_still_hides_host_secrets() {
    let d = ws_with_answer();
    // hermetic: the mask only applies when the host file exists, so the
    // fixture guarantees the precondition on a fresh box and restores state.
    let creds = std::path::Path::new("/root/.git-credentials");
    let preexisting = creds.exists();
    if !preexisting {
        std::fs::write(creds, b"https://canary:canary@example.invalid
").unwrap();
    }
    let restore = |existed: bool| {
        if !existed {
            let _ = std::fs::remove_file(creds);
        }
    };
    // the floor is the whole machine EXCEPT host secret material: /home
    // (glm.key), /mnt (ledger/bundle), /root/.ssh, /root/.git-credentials
    let r = hs_loop::repexec::run_sandboxed(
        d.path(),
        &d.path().join("answer.txt"),
        "ls /home 2>&1; ls /mnt 2>&1; ls -a /root/.ssh 2>&1; wc -c /root/.git-credentials 2>&1",
        30,
    );
    let out = format!(
        "{}{}",
        r["stdout"].as_str().unwrap(),
        r["stderr"].as_str().unwrap()
    );
    assert!(
        out.contains("No such file or directory"),
        "/home must not exist inside: {out}"
    );
    assert!(
        !out.contains("instinct-nvme"),
        "/mnt must not exist inside: {out}"
    );
    assert!(
        !out.contains("authorized_keys"),
        "ssh keys must not be readable: {out}"
    );
    restore(preexisting);
    assert!(
        out.contains("0 /root/.git-credentials"),
        "git-credentials masked to zero bytes: {out}"
    );
    assert!(
        !out.contains("canary"),
        "git-credentials content must not leak: {out}"
    );
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
    let r = hs_loop::repexec::run_sandboxed(
        d.path(),
        &d.path().join("answer.txt"),
        "tail /dev/zero",
        30,
    );
    assert!(
        t0.elapsed().as_secs() < 30,
        "rlimit must kill, not the caller's patience"
    );
    assert!(
        r["exit_code"].as_i64().unwrap_or(0) != 0 || r["timed_out"] == true,
        "bomber must not exit 0: {r}"
    );
}

#[test]
fn sandbox_timeout_and_cleanup_still_hold() {
    let d = ws_with_answer();
    let t0 = std::time::Instant::now();
    let r = hs_loop::repexec::run_sandboxed(d.path(), &d.path().join("answer.txt"), "sleep 30", 2);
    assert!(t0.elapsed().as_secs() < 15);
    assert_eq!(r["timed_out"], true, "{r}");
    let wt = Command::new("git")
        .args(["worktree", "list", "--porcelain"])
        .current_dir(d.path())
        .output()
        .unwrap();
    assert_eq!(
        String::from_utf8_lossy(&wt.stdout)
            .matches("worktree ")
            .count(),
        1,
        "scratch removed after timeout"
    );
}

#[test]
fn sandbox_denies_writes_and_deletes_outside_workspace() {
    // Eric 2026-09-10 (iMessage): no writes, no deletes outside the
    // project workspace. System roots are read-only inside.
    let d = ws_with_answer();
    // host-side sentinel: the delete probe must never target a real file.
    std::fs::write("/usr/local/hs-del-probe", b"probe\n").unwrap();
    let r = hs_loop::repexec::run_sandboxed(
        d.path(),
        &d.path().join("answer.txt"),
        "touch /usr/local/hs-escape.marker 2>&1; echo touch_exit=$?; rm /usr/local/hs-del-probe 2>&1; echo rm_exit=$?; touch /root/hs-escape.marker 2>&1; echo root_touch_exit=$?",
        30,
    );
    let out = format!(
        "{}{}",
        r["stdout"].as_str().unwrap(),
        r["stderr"].as_str().unwrap()
    );
    assert!(
        out.contains("touch_exit=1") || out.contains("Read-only file system"),
        "writes to /usr/local must fail: {out}"
    );
    assert!(
        out.contains("rm_exit=1") || out.contains("Read-only file system"),
        "deletes in /usr/local must fail: {out}"
    );
    assert!(
        std::path::Path::new("/usr/local/hs-del-probe").exists(),
        "host sentinel must survive the sandbox: {out}"
    );
    let _ = std::fs::remove_file("/usr/local/hs-del-probe");
    assert!(
        out.contains("root_touch_exit=1") || out.contains("Read-only file system"),
        "writes to /root must fail: {out}"
    );
    assert!(
        !std::path::Path::new("/usr/local/hs-escape.marker").exists(),
        "no host escape marker"
    );
    assert!(
        !std::path::Path::new("/root/hs-escape.marker").exists(),
        "no host root escape marker"
    );
}

#[test]
fn sandbox_allows_writes_inside_workspace_and_tmp() {
    // The same ruling keeps the workspace fully writable: the mission
    // must be able to build, write, and delete inside /ws and /tmp.
    let d = ws_with_answer();
    let r = hs_loop::repexec::run_sandboxed(
        d.path(),
        &d.path().join("answer.txt"),
        "touch /ws/ws.marker && rm /ws/ws.marker && echo WS-RW; touch /tmp/t.marker && rm /tmp/t.marker && echo TMP-RW",
        30,
    );
    let out = format!(
        "{}{}",
        r["stdout"].as_str().unwrap(),
        r["stderr"].as_str().unwrap()
    );
    assert!(out.contains("WS-RW"), "workspace is writable: {out}");
    assert!(out.contains("TMP-RW"), "tmp scratch is writable: {out}");
}

#[test]
fn sandbox_builds_language_envs_inside_workspace() {
    // Eric 2026-09-10 (iMessage steering): missions MUST be able to create
    // uv/venv environments (and other languages' equivalents) inside the
    // project directory, under confinement. Package caches are writable at
    // their standard locations (blessed by the same ruling).
    let d = ws_with_answer();
    let r = hs_loop::repexec::run_sandboxed(
        d.path(),
        &d.path().join("answer.txt"),
        r#"cd /ws && uv venv .venv 2>&1 && .venv/bin/python -c 'print("UV-VENV-OK")' && uv pip install --python .venv/bin/python --quiet six 2>&1 && .venv/bin/python -c 'import six; print("UV-PIP-OK")' && python3 -m venv .venv2 && .venv2/bin/python -c 'print("PY-VENV-OK")'"#,
        120,
    );
    let out = format!(
        "{}{}",
        r["stdout"].as_str().unwrap(),
        r["stderr"].as_str().unwrap()
    );
    assert!(out.contains("UV-VENV-OK"), "uv venv in workspace: {out}");
    assert!(out.contains("UV-PIP-OK"), "uv pip install in workspace: {out}");
    assert!(out.contains("PY-VENV-OK"), "python venv in workspace: {out}");
}

#[test]
fn sandbox_blesses_standard_toolchain_caches() {
    // Eric 2026-09-10: package caches are fine where the tools need them.
    // The standard locations are writable; the REST of $HOME stays
    // read-only.
    let d = ws_with_answer();
    let r = hs_loop::repexec::run_sandboxed(
        d.path(),
        &d.path().join("answer.txt"),
        r#"for c in /root/.cache /root/.cargo /root/.npm /root/.local /root/go; do touch "$c/.hs-probe" 2>/dev/null && rm "$c/.hs-probe" && echo "RW $c" || echo "RO $c"; done; touch /root/.hs-probe 2>/dev/null && rm /root/.hs-probe && echo "RW /root" || echo "RO /root""#,
        30,
    );
    let out = format!(
        "{}{}",
        r["stdout"].as_str().unwrap(),
        r["stderr"].as_str().unwrap()
    );
    for d2 in ["/root/.cache", "/root/.cargo", "/root/.npm", "/root/.local", "/root/go"] {
        assert!(out.contains(&format!("RW {d2}")), "{d2} writable: {out}");
    }
    assert!(out.contains("RO /root"), "rest of HOME read-only: {out}");
}
