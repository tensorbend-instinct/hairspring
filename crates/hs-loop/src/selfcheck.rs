//! Blind-mode stop authority (Eric 2026-09-07: "no fail to pass - that's
//! cheating"). The agent declares its OWN verification commands, one per
//! line, in `.hs/checks` at the candidate root; checker.run is green only
//! when every declared command exits 0. Ground-truth `FAIL_TO_PASS` never
//! enters the mission - it grades after the fact, outside (hs-swe-run
//! --grade-cmd). Sufficiency of the agent's checks is what the adversarial
//! verifier audits from the transcript.

/// Where the agent's declared checks live inside a candidate worktree.
pub const CHECKS_REL: &str = ".hs/checks";

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
    for c in &cmds {
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
                    "`{c}` exited {:?}\n{}",
                    o.status.code(),
                    tail.trim()
                ));
            }
            Err(e) => failures.push(format!("`{c}` spawn failed: {e}")),
        }
    }
    if failures.is_empty() {
        serde_json::json!({"passed": true, "error": ""})
    } else {
        serde_json::json!({
            "passed": false,
            "error": format!("{}/{} declared checks failed:\n{}", failures.len(), cmds.len(), failures.join("\n---\n")),
        })
    }
}
