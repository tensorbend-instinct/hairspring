//! Gate-2 demo tool plugin "upper" - the capability added by config only.
include!("shared/plugin_sdk.rs");
fn main() {
    serve("upper", "tool", &mut |method, params| match method {
        "tool.call" => {
            serde_json::json!({"output": params["args"]["text"].as_str().unwrap_or("").to_uppercase()})
        }
        _ => serde_json::json!({"$error": "unknown method"}),
    });
}
