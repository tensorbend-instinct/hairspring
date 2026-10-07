//! Control of delegated children (dsh list_agents / send_message / interrupt_agent).
//! State is the swarm registry on disk: `<log_root>/swarm/<child>.spawn.json` marks a child,
//! `.report.json` marks it finished, `.inbox` carries messages the child loop drains at its
//! next step, `.interrupt` makes it stop at its next step boundary.
use serde_json::{json, Value};
use std::path::{Path, PathBuf};

fn dir(log_root: &Path) -> PathBuf {
    log_root.join("swarm")
}

fn clean_id(id: &str) -> bool {
    !id.is_empty() && id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
}

fn owner_alive(marker: &Value) -> bool {
    marker["owner_pid"].as_u64().is_some_and(|p| Path::new(&format!("/proc/{p}")).exists())
}

#[must_use]
pub fn list(log_root: &Path) -> Value {
    let mut agents = vec![];
    if let Ok(rd) = std::fs::read_dir(dir(log_root)) {
        for e in rd.flatten() {
            let n = e.file_name().to_string_lossy().to_string();
            let Some(id) = n.strip_suffix(".spawn.json") else { continue };
            let m: Value = std::fs::read_to_string(e.path()).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or(Value::Null);
            let rep: Value = std::fs::read_to_string(dir(log_root).join(format!("{id}.report.json"))).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or(Value::Null);
            let state = if !rep.is_null() { "done" } else if owner_alive(&m) { "running" } else { "lost" };
            agents.push(json!({"child_stream_id": id, "mission": m["mission"], "model": m["model"], "state": state,
                "passed": rep["passed"], "steps": rep["steps"],
                "interrupt_requested": dir(log_root).join(format!("{id}.interrupt")).exists()}));
        }
    }
    agents.sort_by(|a, b| a["child_stream_id"].as_str().cmp(&b["child_stream_id"].as_str()));
    json!({"ok": true, "agents": agents})
}

fn live_child(log_root: &Path, id: &str) -> Result<(), String> {
    if !clean_id(id) {
        return Err("invalid child id".into());
    }
    let d = dir(log_root);
    if !d.join(format!("{id}.spawn.json")).exists() {
        return Err(format!("unknown child {id}"));
    }
    if d.join(format!("{id}.report.json")).exists() {
        return Err(format!("child {id} already finished"));
    }
    Ok(())
}

#[must_use]
pub fn send(log_root: &Path, child: &str, text: &str) -> Value {
    if text.trim().is_empty() {
        return json!({"$error": "message is empty"});
    }
    if let Err(e) = live_child(log_root, child) {
        return json!({"$error": e});
    }
    let line = text.replace(['\n', '\r'], " ");
    let path = dir(log_root).join(format!("{child}.inbox"));
    use std::io::Write;
    match std::fs::OpenOptions::new().create(true).append(true).open(&path).and_then(|mut f| writeln!(f, "{}", line.trim())) {
        Ok(()) => json!({"ok": true, "queued_for": child}),
        Err(e) => json!({"$error": format!("cannot queue message: {e}")}),
    }
}

#[must_use]
pub fn interrupt(log_root: &Path, child: &str) -> Value {
    if let Err(e) = live_child(log_root, child) {
        return json!({"$error": e});
    }
    match std::fs::write(dir(log_root).join(format!("{child}.interrupt")), "") {
        Ok(()) => json!({"ok": true, "interrupt_requested": child}),
        Err(e) => json!({"$error": format!("cannot request interrupt: {e}")}),
    }
}
