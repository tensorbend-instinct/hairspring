//! Gate-3 bench tool "answer.write": writes the candidate artifact.
include!("shared/sdk.rs");
fn main() {
    serve("answer.write", "tool", &mut |method, params| match method {
        "tool.call" => {
            let path = params["args"]["path"].as_str().unwrap_or("");
            let content = params["args"]["content"].as_str().unwrap_or("");
            match std::fs::write(path, content) {
                Ok(()) => serde_json::json!({"written": true, "path": path}),
                Err(e) => serde_json::json!({"$error": e.to_string()}),
            }
        }
        _ => serde_json::json!({"$error": "unknown method"}),
    });
}
