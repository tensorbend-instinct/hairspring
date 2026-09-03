//! Gate-3 bench tool "checker.run": ground-truth checker for the 24-task
//! family. Tasks 0..18 are feedback-repairable (the verdict names the
//! expected token, the way a compiler error names the fix); 18..24 are
//! unrepairable (the verdict carries no usable signal).
include!("shared/sdk.rs");

fn expected(task_id: &str) -> Option<(usize, String)> {
    let n: usize = task_id.strip_prefix("task-")?.parse().ok()?;
    if n >= 24 {
        return None;
    }
    Some((n, format!("TOKEN-{n}-SECRET")))
}

fn main() {
    serve("checker.run", "tool", &mut |method, params| match method {
        "tool.call" => {
            let a = &params["args"];
            let task_id = a["task_id"].as_str().unwrap_or("");
            let path = a["path"].as_str().unwrap_or("");
            let Some((n, want)) = expected(task_id) else {
                return serde_json::json!({"$error": format!("unknown task {task_id}")});
            };
            let got = std::fs::read_to_string(path).unwrap_or_default();
            if got.trim() == want {
                serde_json::json!({"passed": true, "task_id": task_id})
            } else if n < 18 {
                serde_json::json!({"passed": false, "task_id": task_id,
                    "error": format!("line 1: expected token {want}")})
            } else {
                serde_json::json!({"passed": false, "task_id": task_id,
                    "error": "content mismatch".to_string()})
            }
        }
        _ => serde_json::json!({"$error": "unknown method"}),
    });
}
