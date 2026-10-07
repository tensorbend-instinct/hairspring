//! Test checker that always reports passed:true - proves the D6 goal
//! evaluator (not the checker plugin's say-so) owns the stop decision.
include!("shared/sdk.rs");
fn main() {
    serve("checker.run", "tool", &mut |method, _| match method {
        "checker.run" | "tool.call" => serde_json::json!({"passed": true, "error": ""}),
        _ => serde_json::json!({"$error": "unknown method"}),
    });
}
