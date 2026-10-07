//! Permission ask/auto, enforced in the dispatcher. With `ask` in the file named by
//! `HS_PERMISSION_FILE`, a mutating tool call publishes a question (ask protocol:
//! `q-<n>.json` out, `a-<n>.json` in under `HS_ASK_DIR`) and runs only when the
//! user answers "allow". No answer within `HS_ASK_TIMEOUT_SECS` (default 600) is a denial.
use serde_json::{json, Value};
use std::time::{Duration, Instant};

#[must_use]
pub fn asking() -> bool {
    std::env::var("HS_PERMISSION_FILE")
        .ok()
        .and_then(|p| std::fs::read_to_string(p).ok())
        .is_some_and(|s| s.trim() == "ask")
}

/// `Some(error output)` when the user denied (or never answered) this call.
#[must_use]
pub fn gate(tool: &str, args: &Value) -> Option<Value> {
    if !asking() || matches!(tool, "answer.submit" | "answer.write" | "ask_user_question")
        || !crate::plan_mode::mutates(tool, args)
    {
        return None;
    }
    let dir = std::path::PathBuf::from(std::env::var("HS_ASK_DIR").unwrap_or_else(|_| ".hs/ask".into()));
    if std::fs::create_dir_all(&dir).is_err() {
        return Some(json!({"error": "permission denied: cannot publish the approval question"}));
    }
    let n = (1..).find(|n| !dir.join(format!("q-{n}.json")).exists()).unwrap_or(1);
    let summary: String = args.to_string().chars().take(300).collect();
    let q = json!({"question": format!("Allow {tool}? {summary}"), "options": ["allow", "deny"]});
    if std::fs::write(dir.join(format!("q-{n}.json")), q.to_string()).is_err() {
        return Some(json!({"error": "permission denied: cannot publish the approval question"}));
    }
    let secs: u64 = std::env::var("HS_ASK_TIMEOUT_SECS").ok().and_then(|v| v.parse().ok()).unwrap_or(600);
    let ans = dir.join(format!("a-{n}.json"));
    let t = Instant::now();
    while t.elapsed() < Duration::from_secs(secs) {
        if let Some(v) = std::fs::read_to_string(&ans).ok().and_then(|s| serde_json::from_str::<Value>(&s).ok()) {
            return if v["answer"].as_str() == Some("allow") {
                None
            } else {
                Some(json!({"error": format!("permission denied by the user for {tool}")}))
            };
        }
        std::thread::sleep(Duration::from_millis(25));
    }
    Some(json!({"error": format!("permission denied: no answer within {secs}s for {tool}")}))
}
