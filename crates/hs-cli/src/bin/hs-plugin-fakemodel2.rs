//! Gate-2 demo model plugin "fake-v2" - the model added by config only.
include!("shared/plugin_sdk.rs");
fn main() {
    serve("fake-v2", "model", &mut |method, params| match method {
        "model.call" => {
            let p = params["prompt"].as_str().unwrap_or("");
            serde_json::json!({
                "completion": format!("fake-v2 says: {}", p.to_uppercase()),
                "input_tokens": p.len() / 4 + 1,
                "output_tokens": 11,
                "cost_usd_micros": 2100
            })
        }
        _ => serde_json::json!({"$error": "unknown method"}),
    });
}
