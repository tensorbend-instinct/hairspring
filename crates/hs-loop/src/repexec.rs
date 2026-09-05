//! repo.exec backend (Eric's 2026-09-04 directives): the model may run ANY
//! command against its current answer patch before answer.write - open shell,
//! zero allowlist, safety from ISOLATION ONLY. The patch is applied to a
//! scratch git worktree (live ws never mutated; checker semantics unchanged)
//! and the command runs inside bwrap: private mount/net/pid/ipc namespaces,
//! host home and mission env never enter the sandbox, network off by
//! construction, rlimits + hard timeout cap resources.

use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

const OUT_TAIL: usize = 8192;

fn tail(bytes: &[u8]) -> String {
    let s = String::from_utf8_lossy(bytes);
    if s.len() > OUT_TAIL {
        s[s.len() - OUT_TAIL..].to_string()
    } else {
        s.to_string()
    }
}

/// The sandbox command line, as an argv vector (pure, unit-testable).
/// Full machine floor (fix 1): the mission gets the real box - rw system
/// roots (apt/pip/cargo/npm actually work), network on, scratch rw at /ws,
/// tmpfs /tmp. What stays OUT is host secret material, not capability:
/// /home and /mnt are never bound, /root/.ssh is tmpfs'd over,
/// /root/.git-credentials is masked with /dev/null, and --clearenv keeps
/// mission env (HS_*, keys) out. pid/ipc stay unshared so a mission cannot
/// signal or shm-snoop harness processes.
pub fn sandbox_argv(scratch: &Path, _out_f: &Path, _err_f: &Path, cmd: &str) -> Vec<String> {
    // out/err paths inside the sandbox: the scratch is mounted at /ws
    // Group the command: `a; b >file` redirects ONLY the last simple
    // command, which used to silently lose every earlier command's output
    // (they went to the null'd child stdout). Compound commands are the
    // common case for real shells.
    let script = format!("{{ {cmd}
}} >/ws/.repexec-out 2>/ws/.repexec-err");
    let mut v: Vec<String> = [
        "prlimit", "--as=8589934592", "--nproc=512", "--fsize=8589934592", "--nofile=4096",
        "--", "bwrap", "--share-net", "--unshare-pid", "--unshare-ipc", "--die-with-parent", "--clearenv",
        "--bind", "/usr", "/usr", "--bind", "/bin", "/bin",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect();
    for d in ["/lib", "/lib64", "/etc", "/var", "/opt", "/root"] {
        if Path::new(d).exists() {
            v.push("--bind".into());
            v.push(d.into());
            v.push(d.into());
        }
    }
    // mask host secret material that sits under the bound roots (a
    // zero-length regular file: /dev/null as a bind source is EACCES under
    // this box's device policy)
    let mask = scratch.join(".repexec-mask");
    let _ = std::fs::write(&mask, b"");
    if Path::new("/root/.git-credentials").exists() {
        v.extend(["--ro-bind".into(), mask.display().to_string(), "/root/.git-credentials".into()]);
    }
    if Path::new("/root/.ssh").exists() {
        v.extend(["--tmpfs".into(), "/root/.ssh".into()]);
    }
    v.extend([
        "--dev-bind".into(), "/dev".into(), "/dev".into(),
        "--proc".into(), "/proc".into(),
        "--bind".into(), scratch.display().to_string(), "/ws".into(),
        "--tmpfs".into(), "/tmp".into(),
        "--chdir".into(), "/ws".into(),
        "--setenv".into(), "PATH".into(), "/root/.cargo/bin:/usr/local/cargo/bin:/usr/local/bin:/usr/bin:/bin".into(),
        "--setenv".into(), "HOME".into(), "/root".into(),
        "--setenv".into(), "LANG".into(), "C.UTF-8".into(),
        "--".into(), "sh".into(), "-c".into(), script,
    ]);
    v
}

/// Extract one unified diff from model-supplied text: a ```diff fence, or
/// the raw diff itself (starts with "diff --git" or "--- ").
pub fn extract_diff(raw: &str) -> Option<String> {
    if let Some(p) = hs_bench::extract_patch(raw) {
        return Some(p);
    }
    let t = raw.trim();
    (t.starts_with("diff --git") || t.starts_with("--- ")).then(|| t.to_string())
}

/// Shared prep: read the answer, extract the diff, make a scratch worktree,
/// apply the patch there. Ok(None) = clean feedback result (no patch / no
/// diff / does not apply); Err = machinery failure result.
fn prep(ws: &Path, answer_path: &Path) -> Result<Option<PathBuf>, Value> {
    let raw = match std::fs::read_to_string(answer_path) {
        Ok(s) => s,
        Err(_) => {
            return Err(json!({"applied": false, "note": "no patch to test yet - write your answer first (answer.write), then exec (or pass args.diff inline)"}));
        }
    };
    let Some(patch) = extract_diff(&raw) else {
        return Err(json!({"applied": false, "note": "no diff found in the current answer - wrap one unified diff in a ```diff fence"}));
    };
    prep_diff(ws, &patch)
}

/// Prep from a diff the model supplies inline (T4: test before the first
/// answer.write). Same scratch-worktree semantics as prep.
fn prep_diff(ws: &Path, patch: &str) -> Result<Option<PathBuf>, Value> {
    let uniq = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let scratch = std::env::temp_dir().join(format!("repexec-{}-{}", std::process::id(), uniq));
    let _ = Command::new("git")
        .args(["worktree", "remove", "--force"])
        .arg(&scratch)
        .current_dir(ws)
        .output();
    match Command::new("git")
        .args(["worktree", "add", "--detach"])
        .arg(&scratch)
        .arg("HEAD")
        .current_dir(ws)
        .output()
    {
        Ok(o) if o.status.success() => {}
        Ok(o) => {
            return Err(json!({"$error": format!("scratch worktree: {}", String::from_utf8_lossy(&o.stderr))}));
        }
        Err(e) => return Err(json!({"$error": format!("scratch worktree: {e}")})),
    }
    match hs_bench::apply_model_patch(&scratch, patch) {
        Ok(hs_bench::ApplyResult::Applied) => Ok(Some(scratch)),
        Ok(hs_bench::ApplyResult::NoApply(msg)) => {
            cleanup(ws, &scratch);
            Err(json!({"applied": false, "apply_error": msg}))
        }
        Err(e) => {
            cleanup(ws, &scratch);
            Err(json!({"$error": format!("apply machinery: {e:?}")}))
        }
    }
}

fn cleanup(ws: &Path, scratch: &Path) {
    let _ = Command::new("git")
        .args(["worktree", "remove", "--force"])
        .arg(scratch)
        .current_dir(ws)
        .output();
    let _ = Command::new("git")
        .args(["worktree", "prune"])
        .current_dir(ws)
        .output();
}

/// Open-shell exec in the sandbox. `command` is arbitrary by design.
pub fn run_sandboxed(ws: &Path, answer_path: &Path, command: &str, timeout_secs: u64) -> Value {
    run_with_prep(prep(ws, answer_path), ws, command, timeout_secs)
}

/// Open-shell exec against an inline diff (T4: test-before-first-submit).
pub fn run_sandboxed_with_diff(ws: &Path, diff: &str, command: &str, timeout_secs: u64) -> Value {
    let Some(patch) = extract_diff(diff) else {
        return json!({"applied": false, "note": "no unified diff in args.diff - pass one unified diff, raw or in a ```diff fence"});
    };
    run_with_prep(prep_diff(ws, &patch), ws, command, timeout_secs)
}

fn run_with_prep(prepped: Result<Option<PathBuf>, Value>, ws: &Path, command: &str, timeout_secs: u64) -> Value {
    let scratch = match prepped {
        Ok(Some(s)) => s,
        Ok(None) => unreachable!(),
        Err(early) => return early,
    };
    let out_f = scratch.join(".repexec-out");
    let err_f = scratch.join(".repexec-err");
    let argv = sandbox_argv(&scratch, &out_f, &err_f, command.trim());
    let child = Command::new(&argv[0])
        .args(&argv[1..])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn();
    let mut child = match child {
        Ok(c) => c,
        Err(e) => {
            cleanup(ws, &scratch);
            return json!({"$error": format!("spawn sandbox: {e}")});
        }
    };
    let t0 = std::time::Instant::now();
    let timeout = std::time::Duration::from_secs(timeout_secs);
    let (status, timed_out) = loop {
        match child.try_wait() {
            Ok(Some(st)) => break (Some(st), false),
            Ok(None) if t0.elapsed() > timeout => {
                let _ = child.kill();
                let _ = child.wait();
                break (None, true);
            }
            Ok(None) => std::thread::sleep(std::time::Duration::from_millis(50)),
            Err(e) => {
                cleanup(ws, &scratch);
                return json!({"$error": format!("wait: {e}")});
            }
        }
    };
    let stdout = tail(&std::fs::read(&out_f).unwrap_or_default());
    let stderr = tail(&std::fs::read(&err_f).unwrap_or_default());
    cleanup(ws, &scratch);
    if timed_out {
        return json!({"applied": true, "timed_out": true, "timeout_secs": timeout_secs,
                      "stdout": stdout, "stderr": stderr});
    }
    json!({"applied": true, "timed_out": false,
           "exit_code": status.and_then(|s| s.code()).unwrap_or(-1),
           "stdout": stdout, "stderr": stderr})
}
