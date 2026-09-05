//! D6 goal evaluator: acceptance-constrained stopping (openJiuwen Goal
//! Mode). The stop decision is owned by a VERIFIABLE predicate - the
//! submitted patch applies and the FAIL_TO_PASS command exits 0 inside the
//! bwrap sandbox - never by a checker plugin's say-so (T7).

use serde_json::Value;
use std::path::{Path, PathBuf};

#[derive(Clone)]
pub struct GoalSpec {
    pub ws: PathBuf,
    pub f2p: Vec<String>,
    pub timeout_secs: u64,
}

/// Verify the acceptance predicate against the current answer file.
/// Reuses the repo.exec sandbox: scratch worktree, no host fs, no network.
pub fn verify(goal: &GoalSpec, answer_path: &Path) -> bool {
    let cmd = goal.f2p.join(" && ");
    let r: Value = crate::repexec::run_sandboxed(&goal.ws, answer_path, &cmd, goal.timeout_secs);
    r["applied"].as_bool() == Some(true) && r["exit_code"].as_i64() == Some(0)
}
