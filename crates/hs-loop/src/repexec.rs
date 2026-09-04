//! repo.exec backend (Eric's 2026-09-04 directive): the model may run
//! allowlisted lint/test commands against its CURRENT answer patch before
//! answer.write, like a real SWE agent. The patch is applied to a scratch
//! git worktree - the live workspace is never mutated, so checker semantics
//! are unchanged. A patch that fails to apply comes back as clean
//! applied=false feedback (the A/B/C showed all three arms' first answer
//! was unappliable; this makes that feedback free instead of costing a
//! checker cycle).

use serde_json::{json, Value};
use std::path::Path;
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

pub fn run(
    ws: &Path,
    answer_path: &Path,
    command: &str,
    allowlist: &[String],
    timeout_secs: u64,
) -> Value {
    let cmd = command.trim();
    if !allowlist.iter().any(|p| !p.trim().is_empty() && cmd.starts_with(p.trim())) {
        return json!({"$error": format!("command not in allowlist: {cmd:?}. Allowed prefixes: {allowlist:?}")});
    }
    let raw = match std::fs::read_to_string(answer_path) {
        Ok(s) => s,
        Err(_) => {
            return json!({"applied": false, "note": "no patch to test yet - write your answer first (answer.write), then exec"});
        }
    };
    let Some(patch) = hs_bench::extract_patch(&raw) else {
        return json!({"applied": false, "note": "no diff found in the current answer - wrap one unified diff in a ```diff fence"});
    };

    // scratch worktree off HEAD; live ws untouched
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
    let add = Command::new("git")
        .args(["worktree", "add", "--detach"])
        .arg(&scratch)
        .arg("HEAD")
        .current_dir(ws)
        .output();
    match add {
        Ok(o) if o.status.success() => {}
        Ok(o) => {
            return json!({"$error": format!("scratch worktree: {}", String::from_utf8_lossy(&o.stderr))})
        }
        Err(e) => return json!({"$error": format!("scratch worktree: {e}")}),
    }
    let cleanup = |ws: &Path, scratch: &Path| {
        let _ = Command::new("git")
            .args(["worktree", "remove", "--force"])
            .arg(scratch)
            .current_dir(ws)
            .output();
        let _ = Command::new("git")
            .args(["worktree", "prune"])
            .current_dir(ws)
            .output();
    };

    match hs_bench::apply_model_patch(&scratch, &patch) {
        Ok(hs_bench::ApplyResult::Applied) => {}
        Ok(hs_bench::ApplyResult::NoApply(msg)) => {
            cleanup(ws, &scratch);
            return json!({"applied": false, "apply_error": msg});
        }
        Err(e) => {
            cleanup(ws, &scratch);
            return json!({"$error": format!("apply machinery: {e:?}")});
        }
    }

    let out_f = scratch.join(".repexec-out");
    let err_f = scratch.join(".repexec-err");
    let (of, ef) = match (std::fs::File::create(&out_f), std::fs::File::create(&err_f)) {
        (Ok(a), Ok(b)) => (a, b),
        _ => {
            cleanup(ws, &scratch);
            return json!({"$error": "scratch output files"});
        }
    };
    let child = Command::new("sh")
        .arg("-c")
        .arg(cmd)
        .current_dir(&scratch)
        .stdin(Stdio::null())
        .stdout(Stdio::from(of))
        .stderr(Stdio::from(ef))
        .spawn();
    let mut child = match child {
        Ok(c) => c,
        Err(e) => {
            cleanup(ws, &scratch);
            return json!({"$error": format!("spawn: {e}")});
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
