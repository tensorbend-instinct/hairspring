//! Blind-mode stop authority (Eric 2026-09-07: "no fail to pass - that's
//! cheating"). The agent declares its OWN verification commands, one per
//! line, in `.hs/checks` at the candidate root; checker.run is green only
//! when every declared command exits 0. Ground-truth `FAIL_TO_PASS` never
//! enters the mission - it grades after the fact, outside (hs-swe-run
//! --grade-cmd). Sufficiency of the agent's checks is what the adversarial
//! verifier audits from the transcript.

/// Where the agent's declared checks live inside a candidate worktree.
pub const CHECKS_REL: &str = ".hs/checks";

/// Atomic declare-at-submit channel (RED 2026-09-15, hs-research-repro
/// stream 70ec2565): answer.submit's optional `checks` argument lands here
/// so research-shaped work can declare its verification WITH the submission
/// instead of one step earlier (the goal-verbatim mission prompt never
/// teaches the checks-before-submit ordering; the model learned it from the
/// post-submit "no checks declared" feedback and died at the step cap one
/// resubmit short). Same file, same contract: the checker re-runs exactly
/// these commands - nothing about the declaration is trusted. Refuses an
/// empty declaration rather than dropping it silently; returns the number
/// of declared commands.
pub fn declare_checks(root: &std::path::Path, content: &str) -> Result<u32, String> {
    let cmds = content
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .count();
    if cmds == 0 {
        return Err(format!(
            "empty declaration: pass verification commands, one per line (they land in {CHECKS_REL} and the checker re-runs them)"
        ));
    }
    let dir = root.join(".hs");
    std::fs::create_dir_all(&dir).map_err(|e| format!("create {}: {e}", dir.display()))?;
    let f = dir.join("checks");
    let mut body = content.to_string();
    if !body.ends_with('\n') {
        body.push('\n');
    }
    std::fs::write(&f, body).map_err(|e| format!("write {}: {e}", f.display()))?;
    #[allow(clippy::cast_possible_truncation)]
    Ok(cmds as u32)
}

/// Exit 127/126 means the command could not be run at all: the cause is
/// the environment (PATH/permissions), not the code under test.
fn env_hint(code: Option<i64>) -> &'static str {
    match code {
        Some(127 | 126) => "\n[ENVIRONMENT, not your code: the command was not found or not executable in the checker's environment, which is the same PATH/HOME as your shell. Run `command -v <tool>` in your shell, then install it or declare the check with a full path.]",
        _ => "",
    }
}

/// Hang guard for one declared command, the same mechanism term.exec uses.
const CHECK_TIMEOUT_SECS: u64 = 3600;

/// checker.run verdict for the candidate of `ws`: run every declared
/// command in the candidate, green only when all pass. Feedback names the
/// failing command with its output tail so the loop can repair.
pub fn check(ws: &std::path::Path) -> serde_json::Value {
    // Terminal-bench mode (HS_SELFCHECK_DIRECT=1): no candidate worktree -
    // the agent works on the live machine and .hs/checks lives in the real
    // workdir. Same contract: green only when every declared command passes.
    let direct = std::env::var("HS_SELFCHECK_DIRECT").as_deref() == Ok("1");
    let cand = if direct {
        ws.to_path_buf()
    } else {
        crate::editapply::candidate_dir(ws)
    };
    let f = cand.join(CHECKS_REL);
    let text = match std::fs::read_to_string(&f) {
        Ok(t) => t,
        Err(_) => {
            return serde_json::json!({
                "passed": false,
                "error": format!("no checks declared: write your own verification commands, one per line, to {CHECKS_REL} (they run against your candidate; a submission is only as strong as the checks you declare)"),
            });
        }
    };
    let cmds: Vec<&str> = text
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .collect();
    if cmds.is_empty() {
        return serde_json::json!({
            "passed": false,
            "error": format!("{CHECKS_REL} declares no commands"),
        });
    }
    let mut failures: Vec<String> = vec![];
    // The model's shell (termexec) runs under a scrubbed env: HOME and PATH
    // point into the project root, where its user-site installs live. A
    // declared check must run in THAT env, or an install the model made
    // (pytest under <root>/.local) is invisible here and a green candidate
    // reads red (2026-10-06: 9 min of resubmits). When a project root is
    // configured and the candidate is inside it, run through the same
    // confined spawn; otherwise (legacy/test surfaces) keep the plain shell.
    let confined = crate::projectroot::project_root()
        .filter(|root| cand.canonicalize().map(|c| c.starts_with(root)).unwrap_or(false));
    for c in &cmds {
        if confined.is_some() {
            let r = crate::termexec::run(&cand, c, CHECK_TIMEOUT_SECS);
            if let Some(e) = r["$error"].as_str() {
                failures.push(format!("`{c}` spawn failed: {e}"));
            } else if r["exit_code"].as_i64() != Some(0) || r["timed_out"] == true {
                let mut tail = format!(
                    "{}{}",
                    r["stdout"].as_str().unwrap_or(""),
                    r["stderr"].as_str().unwrap_or("")
                );
                if tail.len() > 2000 {
                    tail = crate::msgfmt::tail_bytes_safe(&tail, 2000);
                }
                let code = if r["timed_out"] == true {
                    format!("timed out after {CHECK_TIMEOUT_SECS}s")
                } else {
                    format!("exited Some({})", r["exit_code"].as_i64().unwrap_or(-1))
                };
                failures.push(format!(
                    "`{c}` {code}\n{}{}",
                    tail.trim(),
                    env_hint(r["exit_code"].as_i64())
                ));
            }
            continue;
        }
        match std::process::Command::new("sh")
            .args(["-c", c])
            .current_dir(&cand)
            .output()
        {
            Ok(o) if o.status.success() => {}
            Ok(o) => {
                let mut tail = String::from_utf8_lossy(&o.stdout).into_owned();
                tail.push_str(&String::from_utf8_lossy(&o.stderr));
                if tail.len() > 2000 {
                    tail = crate::msgfmt::tail_bytes_safe(&tail, 2000);
                }
                failures.push(format!(
                    "`{c}` exited {:?}\n{}{}",
                    o.status.code(),
                    tail.trim(),
                    env_hint(o.status.code().map(i64::from))
                ));
            }
            Err(e) => failures.push(format!("`{c}` spawn failed: {e}")),
        }
    }
    let state = cand.join(".hs/.check_repeat");
    if failures.is_empty() {
        let _ = std::fs::remove_file(&state);
        serde_json::json!({"passed": true, "error": ""})
    } else {
        let mut error = format!(
            "{}/{} declared checks failed:\n{}",
            failures.len(),
            cmds.len(),
            failures.join("\n---\n")
        );
        // Doom-loop signal: the same failure returned again is information
        // the model needs ("nothing you changed touched the cause"). The
        // verdict is unchanged; this only annotates the feedback.
        let sig = {
            let mut h = std::collections::hash_map::DefaultHasher::new();
            std::hash::Hasher::write(&mut h, error.as_bytes());
            std::hash::Hasher::finish(&h)
        };
        let n = match std::fs::read_to_string(&state)
            .ok()
            .and_then(|t| t.split_once(' ').map(|(a, b)| (a.to_string(), b.to_string())))
        {
            Some((prev, cnt)) if prev == sig.to_string() => cnt.trim().parse::<u32>().unwrap_or(1) + 1,
            _ => 1,
        };
        let _ = std::fs::write(&state, format!("{sig} {n}"));
        if n >= 2 {
            error.push_str(&format!(
                "\n---\nREPEAT {n}: this exact failure was returned {n} times in a row. Resubmitting without changing what the failing command depends on cannot pass. Run the failing command yourself in your shell, read its output, and fix that cause before submitting again."
            ));
        }
        serde_json::json!({"passed": false, "error": error})
    }
}
