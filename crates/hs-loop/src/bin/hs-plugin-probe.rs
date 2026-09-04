//! Test tool "probe.read": returns a fixed marker. Proves tool results
//! flow back into the mission context.
include!("shared/sdk.rs");
fn main() {
    serve("probe.read", "tool", &mut |method, params| match method {
        "tool.call" => serde_json::json!({"content": "the marker is MARKER-777"}),
        _ => serde_json::json!({"$error": "unknown method"}),
    });
}
