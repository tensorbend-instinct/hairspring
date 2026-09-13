//! Plugin "policy": the model's self-instruction PROPOSAL path (spec gate
//! 8; bootstrap rule: promotion is gated out-of-mission). Methods:
//!   `policy.propose_prompt` {name, text} -> recorded, versioned, hash-chained
//!     proposal in <run>/`policy_proposals.jsonl` with status "proposed".
//!     Never changes the running mission's prompt.
//! Env: `HS_RUN_DIR` (required) - the mission run directory.
include!("shared/sdk.rs");

fn main() {
    serve(
        "policy.propose_prompt",
        "tool",
        &mut |method, params| match method {
            // The kernel's ToolCall path always sends method "tool.call";
            // matching only the literal name left this tool dead over the
            // wire (RED policy_wire_red, 2026-09-09).
            "policy.propose_prompt" | "tool.call" => {
                // The kernel stamps the mission run dir on every tool.call
                // (the live path); HS_RUN_DIR stays as the batch-script env.
                let dir = match params["run_dir"]
                    .as_str()
                    .map(str::to_owned)
                    .or_else(|| std::env::var("HS_RUN_DIR").ok())
                {
                    Some(d) => d,
                    None => {
                        return serde_json::json!({
                            "$error": "run_dir not set: the kernel stamps it on every tool.call at mission start (HS_RUN_DIR env accepted as fallback)"
                        })
                    }
                };
                let name = params["args"]["name"].as_str().unwrap_or("swe-mission");
                let text = params["args"]["text"].as_str().unwrap_or("");
                if text.trim().is_empty() {
                    return serde_json::json!({"$error": "empty proposal text"});
                }
                match hs_loop::sweprompt::propose_prompt(std::path::Path::new(&dir), name, text) {
                    Ok(rec) => serde_json::json!({
                        "recorded": true,
                        "version": rec.version,
                        "hash": rec.hash,
                        "status": rec.status,
                        "note": "Recorded for gated review; this mission's prompt is unchanged.",
                    }),
                    Err(e) => serde_json::json!({"$error": e}),
                }
            }
            _ => serde_json::json!({"$error": "unknown method"}),
        },
    );
}
