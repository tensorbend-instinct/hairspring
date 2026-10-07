//! Plan mode (dsh parity: exit_plan_mode). While `<workdir>/.hs/plan_mode`
//! exists the loop refuses mutating tools before they reach the kernel; the
//! model reads, searches and writes its plan with the `plan` tool, and the
//! user (or `plan exit`) lifts the gate. The gate lives in the dispatcher so a
//! model cannot talk its way around it.
use serde_json::{json, Value};
use std::path::{Path, PathBuf};

#[must_use]
pub fn state_file(workdir: &Path) -> PathBuf {
    workdir.join(".hs").join("plan_mode")
}

#[must_use]
pub fn active(workdir: &Path) -> bool {
    state_file(workdir).exists()
}

/// Tools that change state outside the model's own notes/plan.
#[must_use]
pub fn mutates(tool: &str, args: &Value) -> bool {
    match tool {
        "term.exec" | "repo.exec" | "edit.apply" | "edit.patch" | "edit.anchor" | "agent.spawn"
        | "answer.submit" | "answer.write" | "world.install" => true,
        "jobs" => matches!(args["op"].as_str(), Some("start" | "kill")),
        _ => false,
    }
}

/// `Some(error output)` when plan mode forbids this call.
#[must_use]
pub fn gate(workdir: &Path, tool: &str, args: &Value) -> Option<Value> {
    (active(workdir) && mutates(tool, args)).then(|| {
        json!({"error": format!(
            "plan mode is on: {tool} is blocked. Read and search freely, write your plan with the plan tool (op=exit with the plan text) and wait for approval to leave plan mode."
        )})
    })
}

#[must_use]
pub fn call(workdir: &Path, args: &Value) -> Value {
    let dir = workdir.join(".hs");
    match args["op"].as_str().unwrap_or("status") {
        "enter" => {
            if std::fs::create_dir_all(&dir).and_then(|()| std::fs::write(state_file(workdir), "1")).is_err() {
                return json!({"$error": "cannot enter plan mode"});
            }
            json!({"ok": true, "plan_mode": true})
        }
        "exit" => {
            let plan = args["plan"].as_str().unwrap_or("");
            if !plan.trim().is_empty() {
                let _ = std::fs::create_dir_all(&dir);
                let _ = std::fs::write(dir.join("plan.md"), plan);
            }
            let _ = std::fs::remove_file(state_file(workdir));
            json!({"ok": true, "plan_mode": false, "plan_saved": !plan.trim().is_empty()})
        }
        "status" => json!({"ok": true, "plan_mode": active(workdir),
            "plan": std::fs::read_to_string(dir.join("plan.md")).unwrap_or_default()}),
        other => json!({"$error": format!("unknown op '{other}'")}),
    }
}
