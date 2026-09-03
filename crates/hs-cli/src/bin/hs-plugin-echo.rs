//! Gate-2 demo tool plugin "echo".
include!("shared/plugin_sdk.rs");
fn main() {
    serve("echo", "tool", &mut |method, params| match method {
        "tool.call" => serde_json::json!({"output": params["args"]["text"].as_str().unwrap_or("")}),
        _ => serde_json::json!({"$error": "unknown method"}),
    });
}
