//! Operating modes (dsh Standard / PTC / Minimal / Creator): each is a named
//! subset of the offered tool surface. `answer.submit` is always kept so a
//! mission can still close through the checker.
use serde_json::Value;

pub const MODES: [&str; 4] = ["standard", "ptc", "minimal", "creator"];

const CORE: [&str; 10] = [
    "repo.read", "repo.search", "repo.exec", "edit.patch", "edit.anchor", "term.exec", "glob",
    "notes.scratch", "answer.submit", "verdict.submit",
];
/// PTC (programmatic tool calling): the core plus the orchestration tools code drives.
const PTC_EXTRA: [&str; 11] = [
    "jobs", "web.fetch", "agent.spawn", "agent.spawn_poll", "agent.fork", "agent.list",
    "agent.send", "agent.interrupt", "workflow", "schedule", "todo",
];
/// Creator: the core plus writing, research and presentation tools.
const CREATOR_EXTRA: [&str; 9] = [
    "present", "read_image", "web.search", "web.fetch", "todo", "plan", "goal",
    "ask_user_question", "memory.recall",
];

/// Whether `mode` offers tool `name`. `None` for an unknown mode.
#[must_use]
pub fn allows(mode: &str, name: &str) -> Option<bool> {
    let in_core = CORE.contains(&name);
    match mode {
        "standard" => Some(true),
        "minimal" => Some(in_core),
        "ptc" => Some(in_core || PTC_EXTRA.contains(&name)),
        "creator" => Some(in_core || CREATOR_EXTRA.contains(&name)),
        _ => None,
    }
}

/// The tool schemas `mode` offers, or `None` for an unknown mode.
#[must_use]
pub fn filter(mode: &str, tools: &Value) -> Option<Value> {
    allows(mode, "answer.submit")?;
    let arr = tools.as_array()?;
    Some(Value::Array(
        arr.iter()
            .filter(|t| t["function"]["name"].as_str().is_some_and(|n| allows(mode, n) == Some(true)))
            .cloned()
            .collect(),
    ))
}
