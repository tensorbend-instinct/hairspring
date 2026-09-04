//! Test tool "bigread.read": returns a ~30KB page of filler tagged with its
//! page number, so a handful of calls overflow the transcript window and
//! trip compaction pressure in tests.
include!("shared/sdk.rs");
fn main() {
    serve("bigread.read", "tool", &mut |method, params| match method {
        "tool.call" => {
            let page = params["args"]["page"].as_u64().unwrap_or(0);
            serde_json::json!({"content": format!("PAGE-{} {}", page, "x".repeat(30_000))})
        }
        _ => serde_json::json!({"$error": "unknown method"}),
    });
}
