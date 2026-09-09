//! SWE mission tool "notes.scratch": persistent model-writable notes.
//! args: {op:"write"|"append"|"read", content?}. Storage: `HS_SCRATCH_FILE`
//! (hs-swe-run sets it to the run's work dir, so notes outlive the mission's
//! context window and plugin restarts).
include!("shared/sdk.rs");
fn main() {
    serve(
        "notes.scratch",
        "tool",
        &mut |method, params| match method {
            "tool.call" => {
                let path = match std::env::var("HS_SCRATCH_FILE") {
                    Ok(p) => p,
                    Err(_) => return serde_json::json!({"$error": "HS_SCRATCH_FILE not set"}),
                };
                let op = params["args"]["op"].as_str().unwrap_or("read");
                match op {
                    "write" | "append" => {
                        let content = params["args"]["content"].as_str().unwrap_or("");
                        let r = if op == "write" {
                            std::fs::write(&path, content)
                        } else {
                            use std::io::Write;
                            std::fs::OpenOptions::new()
                                .create(true)
                                .append(true)
                                .open(&path)
                                .and_then(|mut f| f.write_all(content.as_bytes()))
                        };
                        match r {
                            Ok(()) => serde_json::json!({"ok": true, "op": op}),
                            Err(e) => serde_json::json!({"$error": format!("notes {op}: {e}")}),
                        }
                    }
                    "read" => serde_json::json!({"ok": true,
                    "content": std::fs::read_to_string(&path).unwrap_or_default()}),
                    other => serde_json::json!({"$error": format!("unknown op '{other}'")}),
                }
            }
            _ => serde_json::json!({"$error": "unknown method"}),
        },
    );
}
