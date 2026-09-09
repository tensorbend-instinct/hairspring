//! Test fixture (`tool_cost_red)`: a tool that reports a nonzero cost in its
//! output body, so the mission log's canonical per-event cost field can be
//! checked against what the tool reported.
include!("shared/sdk.rs");
fn main() {
    serve("costtool.probe", "tool", &mut |method, _params| match method {
        "tool.call" => serde_json::json!({"probed": true, "cost_usd_micros": 1234}),
        _ => serde_json::json!({"$error": "unknown method"}),
    });
}
