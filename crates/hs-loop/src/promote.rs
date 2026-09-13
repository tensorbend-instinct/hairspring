//! Promotion driver glue (spec gate 8 / D7, Eric 2026-09-13): the
//! out-of-mission path that takes recorded policy proposals and runs them
//! through evolve::evaluate_candidate - benched against the parent on
//! held-out tasks, journaled, promoted into the [prompts] overlay the
//! live surfaces load, or rewound. The runner is injected: fixture
//! missions in tests and the free proof, real SWE missions through
//! hs-swe-run for the paid cycle.

use crate::sweprompt::{self, PolicyOverlay, ProposalRecord};
use std::path::{Path, PathBuf};

/// The latest recorded proposal in a policy_proposals.jsonl log: the
/// candidate the driver benches.
pub fn latest_proposal(proposals_path: &Path) -> Result<ProposalRecord, String> {
    let text = std::fs::read_to_string(proposals_path)
        .map_err(|e| format!("proposals {}: {e}", proposals_path.display()))?;
    let mut last = None;
    for line in text.lines().filter(|l| !l.trim().is_empty()) {
        let r: ProposalRecord = serde_json::from_str(line)
            .map_err(|e| format!("corrupt proposal log {}: {e}", proposals_path.display()))?;
        last = Some(r);
    }
    last.ok_or_else(|| format!("no proposals in {}", proposals_path.display()))
}

/// One specific proposal version from the log.
pub fn proposal_at(proposals_path: &Path, version: u64) -> Result<ProposalRecord, String> {
    let text = std::fs::read_to_string(proposals_path)
        .map_err(|e| format!("proposals {}: {e}", proposals_path.display()))?;
    for line in text.lines().filter(|l| !l.trim().is_empty()) {
        let r: ProposalRecord = serde_json::from_str(line)
            .map_err(|e| format!("corrupt proposal log {}: {e}", proposals_path.display()))?;
        if r.version == version {
            return Ok(r);
        }
    }
    Err(format!(
        "no proposal version {version} in {}",
        proposals_path.display()
    ))
}

/// The parent's current template for `name` from the live overlay;
/// None = the builtin template.
pub fn parent_template(overlay_path: &Path, name: &str) -> Result<Option<String>, String> {
    if !overlay_path.exists() {
        return Ok(None);
    }
    let p = sweprompt::load_policy_overlay(overlay_path)?;
    Ok(p.prompts.get(name).cloned())
}

/// Render an overlay TOML: the current overlay's prompts with `name`
/// replaced by `text` (every other prompt survives).
#[must_use]
pub fn render_overlay(current: Option<&PolicyOverlay>, name: &str, text: &str) -> String {
    let mut prompts = current.map_or_else(Default::default, |c| c.prompts.clone());
    prompts.insert(name.to_string(), text.to_string());
    let mut out = String::from("[prompts]\n");
    for (k, v) in &prompts {
        out.push_str(&format!("{k} = {}\n", toml::Value::String(v.clone())));
    }
    out
}

/// One task id per line (SWE instance ids for the paid runner, fixture
/// task names for the free one).
pub fn load_task_list(path: &Path) -> Result<Vec<String>, String> {
    let text = std::fs::read_to_string(path)
        .map_err(|e| format!("task list {}: {e}", path.display()))?;
    let tasks: Vec<String> = text
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .map(str::to_owned)
        .collect();
    if tasks.is_empty() {
        return Err(format!("task list {} is empty", path.display()));
    }
    Ok(tasks)
}

/// The promotion journal lives next to the canonical overlay by default.
#[must_use]
pub fn default_journal_path() -> PathBuf {
    sweprompt::config_dir().join("policy_journal.jsonl")
}
