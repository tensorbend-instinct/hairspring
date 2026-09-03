//! Gate-2 demo model plugin "fake-v1" (deterministic, no API spend).
include!("shared/plugin_sdk.rs");
fn main() {
    serve("fake-v1", "model", &mut |method, params| match method {
        "model.call" => {
            let p = params["prompt"].as_str().unwrap_or("");
            serde_json::json!({
                "completion": format!("fake-v1 says: {}", p),
                "input_tokens": p.len() / 4 + 1,
                "output_tokens": 9,
                "cost_usd_micros": 800
            })
        }
        _ => serde_json::json!({"$error": "unknown method"}),
    });
}
