//! Test model "seqmodel": scripted sequence for loop-mechanics tests.
//! Step 1 calls probe.read; later steps write TOKEN-0-SECRET only if the
//! probe's marker reached the prompt - isolates "tool results must enter
//! the next step's context" from model quality.
include!("shared/sdk.rs");
fn main() {
    serve("seqmodel", "model", &mut |method, params| match method {
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
            let completion = if attempt == 1 {
                serde_json::json!({"tool":"probe.read","args":{"path":"x"}})
            } else if attempt == 2 {
                serde_json::json!({"tool":"probe.read","args":{"path":"y"}})
            } else if prompt.contains("MARKER-777") {
                serde_json::json!({"tool":"answer.write","args":{"path":path,"content":"TOKEN-0-SECRET"}})
            } else {
                serde_json::json!({"tool":"answer.write","args":{"path":path,"content":"blind-wrong"}})
            };
            serde_json::json!({
                "completion": completion.to_string(),
                "input_tokens": prompt.len() / 4 + 1,
                "output_tokens": 12,
                "cost_usd_micros": 900
            })
        }
        _ => serde_json::json!({"$error": "unknown method"}),
    });
}
