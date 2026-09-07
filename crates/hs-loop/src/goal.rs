//! D6 goal evaluator: acceptance-constrained stopping (openJiuwen Goal
//! Mode). The stop decision is owned by a VERIFIABLE predicate - the
//! submitted patch applies and the FAIL_TO_PASS command exits 0 - never by
//! a checker plugin's say-so (T7). f2p commands are harness-generated
//! (ops/subset/run_subset_par.py writes f2p.sh; HS_SWE_F2P carries it), so
//! they run HOST-side in a scratch worktree (repexec::run_host), at the
//! same exec location as the standalone checker: the bwrap sandbox exists
//! to contain MODEL commands and never binds /home, where mission venvs
//! live (forensic item 1, 2026-09-06: sandboxed goal evals were exit 127
//! env_limited in 100% of sessions).

use serde_json::Value;
use std::path::{Path, PathBuf};

#[derive(Clone)]
pub struct GoalSpec {
    pub ws: PathBuf,
    pub f2p: Vec<String>,
    pub timeout_secs: u64,
}

/// Post-A7: the stop signal is richer than green/red. An f2p run that
/// fails because the exec sandbox lacks the tool (exit 127, "command not
/// found") is ENV-LIMITED - the evaluator cannot judge, and the verdict
/// path must say so instead of reporting a plain red (A7: 22 steps /
/// $1.46 vetoed on a red that was only "no pytest in the sandbox").
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GoalVerdict {
    Pass,
    Fail,
    EnvLimited(String),
}

/// Classify an exec-sandbox result as an environmental failure: the f2p
/// command never really ran because the tool itself is missing. Returns
/// the human reason (naming the missing tool) when so. A nonzero exit
/// from a REAL test run is a genuine red, never env-limited.
pub fn classify_env_failure(r: &Value) -> Option<String> {
    if r["exit_code"].as_i64() != Some(127) {
        return None;
    }
    let err = r["stderr"].as_str().unwrap_or("");
    for line in err.lines() {
        if let Some(pos) = line.find(": command not found") {
            let tool = line[..pos]
                .split_whitespace()
                .last()
                .unwrap_or("<unknown>")
                .trim_matches(':');
            return Some(format!("sandbox lacks {tool} (exit 127)"));
        }
        if line.contains("No such file or directory") {
            return Some(format!("sandbox missing file: {}", line.trim()));
        }
    }
    Some("exit 127 (command not found)".to_string())
}

/// Richer verify: EnvLimited when f2p cannot run at all (missing venv
/// tool, exit 127), so the caller records the limitation explicitly
/// instead of a bare red. Runs HOST-side: these are harness-fixed
/// commands, not model input.
pub fn verify_verdict(goal: &GoalSpec, answer_path: &Path) -> GoalVerdict {
    let cmd = goal.f2p.join(" && ");
    let r: Value = crate::repexec::run_host(&goal.ws, answer_path, &cmd, goal.timeout_secs);
    if r["applied"].as_bool() == Some(true) && r["exit_code"].as_i64() == Some(0) {
        return GoalVerdict::Pass;
    }
    if let Some(reason) = classify_env_failure(&r) {
        return GoalVerdict::EnvLimited(reason);
    }
    GoalVerdict::Fail
}

/// Verify the acceptance predicate against the current answer file.
/// Host-side scratch worktree (run_host): same exec location as the checker.
pub fn verify(goal: &GoalSpec, answer_path: &Path) -> bool {
    verify_verdict(goal, answer_path) == GoalVerdict::Pass
}
