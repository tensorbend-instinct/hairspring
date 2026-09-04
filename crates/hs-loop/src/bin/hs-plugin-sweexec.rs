//! Test model "sweexec": drives repo.exec through the real loop.
//! Attempt 1: answer.write a deliberately corrupt patch (bad framing).
//! Attempt 2: repo.exec the allowlisted check against that patch.
//! Attempt 3: answer.write the gold patch (checker passes, mission ends).
include!("shared/sdk.rs");
fn main() {
    serve("sweexec", "model", &mut |method, params| match method {
        "model.call" => {
            let prompt = params["prompt"].as_str().unwrap_or("");
            let path = prompt
                .lines()
                .find_map(|l| l.strip_prefix("ANSWER_PATH: "))
                .unwrap_or("")
                .to_string();
            let attempt: usize = prompt
                .lines()
                .find_map(|l| l.strip_prefix("ATTEMPT: "))
                .and_then(|v| v.parse().ok())
                .unwrap_or(1);
            let completion = match attempt {
                1 => serde_json::json!({"tool":"answer.write","args":{"path":path,
                    "content":"```diff\n@@ -1 +1 @@\n-broken\n+fixed\n```"}}),
                2 => serde_json::json!({"tool":"repo.exec","args":{"command":"sh check.sh","path":path}}),
                _ => {
                    let gold = std::env::var("HS_SWE_GOLD_PATCH_FILE")
                        .ok()
                        .and_then(|f| std::fs::read_to_string(f).ok())
                        .unwrap_or_default();
                    serde_json::json!({"tool":"answer.write","args":{"path":path,
                        "content":format!("```diff\n{gold}\n```")}})
                }
            };
            serde_json::json!({
                "completion": completion.to_string(),
                "input_tokens": prompt.len() / 4 + 1,
                "output_tokens": 12,
                "cost_usd_micros": 900
            })
        }
        _ => serde_json::json!({"$error": "unknown method"}),
    })
}
