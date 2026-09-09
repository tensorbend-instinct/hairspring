//! Terminal-bench mission tool "term.exec": run a command DIRECTLY in the
//! task container's workdir. The container is the sandbox - no bwrap, no
//! per-call filesystem copy: files, installs, and services persist between
//! calls, which is exactly the machine state terminal-bench grades after the
//! agent finishes. Hard timeout enforced (kill on expiry).

use serde_json::Value;
use std::os::unix::process::CommandExt;
use std::process::Stdio;
use std::time::{Duration, Instant};

fn tail(s: String, n: usize) -> String {
    if s.len() > n {
        s[s.len() - n..].to_string()
    } else {
        s
    }
}

#[must_use]
pub fn run(workdir: &std::path::Path, command: &str, timeout_secs: u64) -> Value {
    if command.trim().is_empty() {
        return serde_json::json!({"$error": "pass command: a bash command line"});
    }
    // The command runs in its OWN process group: a timeout must kill the
    // whole tree. Killing only the wrapper leaves orphaned grandchildren
    // holding the stdout/stderr pipes open, and wait_with_output then
    // blocks until THEY exit (observed live 2026-09-07: `grep -r X /`
    // orphaned by a timed-out call burned 30 min per call at 99% CPU while
    // the mission thread sat in wait_with_output).
    let mut child = match std::process::Command::new("bash")
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
