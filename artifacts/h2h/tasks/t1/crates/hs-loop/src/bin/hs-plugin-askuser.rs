//! Tool "ask_user_question": publish a question for the surface (TUI/app) and
//! block for the answer. Files under HS_ASK_DIR: q-<n>.json out, a-<n>.json in.
include!("shared/sdk.rs");
use std::time::{Duration, Instant};

fn main() {
    serve("ask_user_question", "tool", &mut |method, params| match method {
        "tool.call" => {
            let Some(q) = params["args"]["question"].as_str().filter(|s| !s.trim().is_empty()) else {
                return serde_json::json!({"$error": "question is required"});
            };
            let dir = std::path::PathBuf::from(std::env::var("HS_ASK_DIR").unwrap_or_else(|_| ".hs/ask".into()));
            if std::fs::create_dir_all(&dir).is_err() {
                return serde_json::json!({"$error": "cannot create ask dir"});
            }
            let n = (1..).find(|n| !dir.join(format!("q-{n}.json")).exists()).unwrap_or(1);
            let body = serde_json::json!({"question": q, "options": params["args"]["options"]});
            if std::fs::write(dir.join(format!("q-{n}.json")), body.to_string()).is_err() {
                return serde_json::json!({"$error": "cannot publish question"});
            }
            let secs: u64 = std::env::var("HS_ASK_TIMEOUT_SECS").ok().and_then(|v| v.parse().ok()).unwrap_or(600);
            let ans = dir.join(format!("a-{n}.json"));
            let t = Instant::now();
            while t.elapsed() < Duration::from_secs(secs) {
                if let Ok(s) = std::fs::read_to_string(&ans) {
                    if let Ok(v) = serde_json::from_str::<serde_json::Value>(&s) {
                        return serde_json::json!({"ok": true, "answer": v["answer"]});
                    }
                }
                std::thread::sleep(Duration::from_millis(50));
            }
            serde_json::json!({"$error": format!("no answer from the user within {secs}s; continue with your best assumption and say so")})
        }
        _ => serde_json::json!({"$error": "unknown method"}),
    });
}
