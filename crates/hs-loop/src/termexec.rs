//! Terminal-bench mission tool "term.exec": run a command DIRECTLY in the
//! mission workdir. No per-call filesystem copy: files, installs, and
//! services persist between calls, which is exactly the machine state
//! terminal-bench grades after the agent finishes. Hard timeout enforced
//! (kill on expiry).
//!
//! Confinement (mission isolation, 2026-09-10): when the harness exports
//! HS_PROJECT_ROOT (hs-repl always does - `--project-dir` or the session
//! work area by default) the command runs inside a bwrap namespace where
//! the project root is the ONLY host filesystem bound read-write: /usr,
//! /etc and the /bin|/lib symlinks are read-only binds, /tmp is a fresh
//! tmpfs scratch, the environment is scrubbed to a minimal PATH/HOME (no
//! harness HS_* config, no provider keys), and the network stays up -
//! package installs are part of the terminal-bench contract. Hostile
//! absolute reads and `..` escapes find nothing because nothing outside
//! the root is mounted. The old "the container is the sandbox" assumption
//! held on the one-container-per-task bench rig; in shared-box REPL mode
//! it let missions read every harness file on the host. Fail-closed: no
//! bwrap, no run.
//!
//! Two surfaces: `run` for the AUTHORING agent (root inside the task
//! container - the agent is supposed to mutate), `run_readonly` for the
//! VERIFIER (the independent critic): the command runs as uid/gid `nobody`
//! with the supplementary group list cleared, so root-owned task files are
//! read-only BY MECHANISM (deep pass 2026-09-09: the critic's prompt claimed
//! read-only while the shell ran as unrestricted root). Fail-closed: a
//! non-root caller cannot drop privileges, so the call refuses instead of
//! running unenforced.

use serde_json::Value;
use std::os::unix::process::CommandExt;
use std::process::Stdio;
use std::time::{Duration, Instant};

fn tail(s: String, n: usize) -> String {
    crate::msgfmt::tail_bytes_safe(&s, n)
}

/// uid/gid of the verifier's unprivileged identity.
const NOBODY: u32 = 65534;

/// Effective uid from /proc (dependency-free geteuid). Unknown reads as
/// NOT root, which is the fail-closed answer for `run_readonly`.
fn euid() -> u32 {
    std::fs::read_to_string("/proc/self/status")
        .ok()
        .and_then(|s| {
            s.lines()
                .find(|l| l.starts_with("Uid:"))
                .and_then(|l| l.split_whitespace().nth(2))
                .and_then(|v| v.parse().ok())
        })
        .unwrap_or(u32::MAX)
}

/// Shared tail of both surfaces: own process group, hard timeout that
/// kills the whole tree, bounded output.
fn collect(mut child: std::process::Child, timeout_secs: u64) -> Value {
    // The command runs in its OWN process group: a timeout must kill the
    // whole tree. Killing only the wrapper leaves orphaned grandchildren
    // holding the stdout/stderr pipes open, and wait_with_output then
    // blocks until THEY exit (observed live 2026-09-07: `grep -r X /`
    // orphaned by a timed-out call burned 30 min per call at 99% CPU while
    // the mission thread sat in wait_with_output).
    let deadline = Instant::now() + Duration::from_secs(timeout_secs);
    let timed_out = loop {
        match child.try_wait() {
            Ok(Some(_)) => break false,
            Ok(None) => {
                if Instant::now() >= deadline {
                    // Group kill first (grandchildren release the pipes),
                    // then the direct child as belt-and-braces. bash's
                    // builtin kill keeps this dependency-free.
                    let pgid = child.id();
                    let _ = std::process::Command::new("bash")
                        .args(["-c", &format!("kill -KILL -- -{pgid} 2>/dev/null")])
                        .status();
                    let _ = child.kill();
                    let _ = child.wait();
                    break true;
                }
                std::thread::sleep(Duration::from_millis(50));
            }
            Err(e) => return serde_json::json!({"$error": format!("wait: {e}")}),
        }
    };
    let out = match child.wait_with_output() {
        Ok(o) => o,
        Err(e) => return serde_json::json!({"$error": format!("output: {e}")}),
    };
    serde_json::json!({
        "exit_code": out.status.code().unwrap_or(-1),
        "timed_out": timed_out,
        "stdout": tail(String::from_utf8_lossy(&out.stdout).into_owned(), 6000),
        "stderr": tail(String::from_utf8_lossy(&out.stderr).into_owned(), 3000),
    })
}

/// Confined spawn: see the module doc. `identity` drops to uid/gid
/// nobody for the verifier surface. The bwrap binary missing is a
/// fail-CLOSED error - an unconfined run is never the fallback.
fn spawn_confined(
    root: &std::path::Path,
    workdir: &std::path::Path,
    command: &str,
    identity: Option<(u32, u32)>,
) -> Result<std::process::Child, Value> {
    let root_s = root.to_string_lossy().into_owned();
    let mut cmd = std::process::Command::new("bwrap");
    cmd.args([
        "--unshare-user",
        "--unshare-pid",
        "--unshare-uts",
        "--unshare-ipc",
        "--unshare-cgroup",
        "--die-with-parent",
        "--clearenv",
        "--setenv",
        "PATH",
        "/usr/bin:/bin",
        "--setenv",
        "HOME",
        "/tmp",
        "--ro-bind",
        "/usr",
        "/usr",
        "--symlink",
        "usr/bin",
        "/bin",
        "--symlink",
        "usr/lib",
        "/lib",
        "--symlink",
        "usr/lib64",
        "/lib64",
        "--ro-bind",
        "/etc",
        "/etc",
        "--dev",
        "/dev",
        "--proc",
        "/proc",
        "--tmpfs",
        "/tmp",
        "--bind",
        &root_s,
        &root_s,
    ]);
    if let Some((uid, gid)) = identity {
        cmd.args(["--uid", &uid.to_string(), "--gid", &gid.to_string()]);
    }
    cmd.args(["--chdir", &workdir.to_string_lossy(), "bash", "-c", command])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .process_group(0);
    cmd.spawn().map_err(|e| {
        serde_json::json!({"$error": format!(
            "bwrap sandbox unavailable ({e}) - refusing to run unconfined"
        )})
    })
}

#[must_use]
pub fn run(workdir: &std::path::Path, command: &str, timeout_secs: u64) -> Value {
    if command.trim().is_empty() {
        return serde_json::json!({"$error": "pass command: a bash command line"});
    }
    if let Some(root) = crate::projectroot::project_root() {
        let workdir = match crate::projectroot::confine_existing(workdir, "term.exec workdir") {
            Ok(w) => w,
            Err(e) => return e,
        };
        let child = match spawn_confined(&root, &workdir, command, None) {
            Ok(c) => c,
            Err(e) => return e,
        };
        return collect(child, timeout_secs);
    }
    let child = match std::process::Command::new("bash")
        .args(["-c", command])
        .current_dir(workdir)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .process_group(0)
        .spawn()
    {
        Ok(c) => c,
        Err(e) => return serde_json::json!({"$error": format!("spawn: {e}")}),
    };
    collect(child, timeout_secs)
}

/// The verifier's surface: identical discipline to `run`, but the command
/// executes as uid/gid `nobody` with the supplementary group list cleared,
/// so it can read world-readable task state and write /tmp scratch - and
/// CANNOT modify, move, or delete root-owned task files, whatever the
/// (model-authored) command line says. Enforcement is positional, not a
/// prompt promise.
#[must_use]
pub fn run_readonly(workdir: &std::path::Path, command: &str, timeout_secs: u64) -> Value {
    if command.trim().is_empty() {
        return serde_json::json!({"$error": "pass command: a bash command line"});
    }
    if let Some(root) = crate::projectroot::project_root() {
        // Same confinement as `run`, executed as nobody inside the
        // namespace: the verifier's commands are model-authored too, so
        // they get the same mechanical boundary. bwrap's --uid/--gid also
        // clears the supplementary group list (single group in the new
        // userns), and the root-only requirement of the legacy path does
        // not apply - the userns provides the identity.
        let workdir = match crate::projectroot::confine_existing(workdir, "verify workdir") {
            Ok(w) => w,
            Err(e) => return e,
        };
        let child = match spawn_confined(&root, &workdir, command, Some((NOBODY, NOBODY))) {
            Ok(c) => c,
            Err(e) => return e,
        };
        return collect(child, timeout_secs);
    }
    if euid() != 0 {
        return serde_json::json!({"$error": "run_readonly requires root to drop privileges (setuid nobody); refusing to run unenforced"});
    }
    let mut cmd = std::process::Command::new("bash");
    cmd.args(["-c", command])
        .current_dir(workdir)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .process_group(0)
        .env("HOME", "/tmp")
        .gid(NOBODY)
        .uid(NOBODY);
    // std's uid/gid setup also clears the SUPPLEMENTARY group list
    // (setgroups(0, NULL) between setgid and setuid - verified by strace
    // on this toolchain), so group-0 write bits do not survive the drop.
    let child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) => return serde_json::json!({"$error": format!("spawn: {e}")}),
    };
    collect(child, timeout_secs)
}
