//! Test fixture (`tool_cost_red)`: a model that calls costtool.probe on
//! attempt 1 and declares done on attempt 2, so a one-tool mission exists
//! for the canonical-cost assertion.
include!("shared/sdk.rs");
fn main() {
    serve("costmodel", "model", &mut |method, params| match method {
        "model.call" => {
            let prompt = params["prompt"].as_str().unwrap_or("");
            let attempt: usize = prompt
                .lines()
                .find_map(|l| l.strip_prefix("ATTEMPT: "))
                .and_then(|v| v.parse().ok())
                .unwrap_or(1);
            let completion = if attempt >= 2 {
                serde_json::json!({"done": true})
            } else {
                serde_json::json!({"tool": "costtool.probe", "args": {}})
            };
            serde_json::json!({
                "completion": completion.to_string(),
                "input_tokens": 1,
                "output_tokens": 1,
                "cost_usd_micros": 0
            })
        }
        _ => serde_json::json!({"$error": "unknown method"}),
    });
}
