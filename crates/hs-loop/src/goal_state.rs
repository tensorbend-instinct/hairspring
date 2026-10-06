//! Long-running task goal (dsh parity: create_goal / get_goal / update_goal).
//! One goal per workdir in `<workdir>/.hs/goal.json`: objective, optional
//! acceptance text, status active|done|blocked, append-only notes. This is the
//! model's own tracker; the mission is still closed only by the checker.
use serde_json::{json, Value};
use std::path::Path;

const STATUSES: [&str; 3] = ["active", "done", "blocked"];

fn file(workdir: &Path) -> std::path::PathBuf {
    workdir.join(".hs").join("goal.json")
}

fn load(workdir: &Path) -> Option<Value> {
    serde_json::from_str(&std::fs::read_to_string(file(workdir)).ok()?).ok()
}

fn save(workdir: &Path, v: &Value) -> bool {
    std::fs::create_dir_all(workdir.join(".hs")).is_ok() && std::fs::write(file(workdir), v.to_string()).is_ok()
}

#[must_use]
pub fn call(workdir: &Path, args: &Value) -> Value {
    match args["op"].as_str().unwrap_or("get") {
        "create" => {
            let Some(obj) = args["objective"].as_str().filter(|s| !s.trim().is_empty()) else {
                return json!({"$error": "create needs objective"});
            };
            if load(workdir).is_some_and(|g| g["status"] == "active") {
                return json!({"$error": "an active goal already exists; update it to done/blocked first"});
            }
            let g = json!({"objective": obj, "acceptance": args["acceptance"], "status": "active", "notes": []});
            if !save(workdir, &g) {
                return json!({"$error": "cannot save goal"});
            }
            json!({"ok": true, "goal": g})
        }
        "get" => match load(workdir) {
            Some(g) => json!({"ok": true, "goal": g}),
            None => json!({"ok": true, "goal": null}),
        },
        "update" => {
            let Some(mut g) = load(workdir) else {
                return json!({"$error": "no goal; create one first"});
            };
            if let Some(s) = args["status"].as_str() {
                if !STATUSES.contains(&s) {
                    return json!({"$error": format!("status must be one of {STATUSES:?}")});
                }
                g["status"] = json!(s);
            }
            if let Some(n) = args["note"].as_str().filter(|s| !s.trim().is_empty()) {
                if let Some(a) = g["notes"].as_array_mut() {
                    a.push(json!(n));
                }
            }
            if !save(workdir, &g) {
                return json!({"$error": "cannot save goal"});
            }
            json!({"ok": true, "goal": g})
        }
        other => json!({"$error": format!("unknown op '{other}'")}),
    }
}
