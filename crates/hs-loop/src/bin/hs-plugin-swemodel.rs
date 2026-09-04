//! Scripted SWE-mission model for the offline integration proof. Attempt 1
//! emits prose (no diff). Once the checker's feedback arrives, it writes the
//! gold patch from HS_SWE_GOLD_PATCH_FILE as a fenced diff inside the
//! required JSON tool call. Isolates the harness from model quality.
include!("shared/sdk.rs");

fn main() {
    serve("swemodel", "model", &mut |method, params| match method {
        "model.call" => {
            let prompt = params["prompt"].as_str().unwrap_or("");
            let path = prompt
                .lines()
                .find_map(|l| l.strip_prefix("ANSWER_PATH: "))
                .unwrap_or("")
                .to_string();
            let has_feedback = prompt.contains("FEEDBACK:");
            let content = if has_feedback {
                let gold = std::env::var("HS_SWE_GOLD_PATCH_FILE")
                    .ok()
                    .and_then(|f| std::fs::read_to_string(f).ok())
                    .unwrap_or_default();
                format!("```diff\n{gold}\n```")
            } else {
                "I have not looked at the code yet. No patch.".to_string()
            };
            let completion = serde_json::json!({
                "tool": "answer.write",
                "args": {"path": path, "content": content}
            });
            serde_json::json!({
                "completion": completion.to_string(),
                "input_tokens": prompt.len() / 4 + 1,
                "output_tokens": content.len() / 4 + 1,
                "cost_usd_micros": 900
            })
        }
        _ => serde_json::json!({"$error": "unknown method"}),
    });
}
