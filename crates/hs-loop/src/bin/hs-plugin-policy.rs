//! Plugin "policy": the model's self-instruction PROPOSAL path (spec gate
//! 8; bootstrap rule: promotion is gated out-of-mission). Methods:
//!   policy.propose_prompt {name, text} -> recorded, versioned, hash-chained
//!     proposal in <run>/policy_proposals.jsonl with status "proposed".
//!     Never changes the running mission's prompt.
//! Env: HS_RUN_DIR (required) - the mission run directory.
include!("shared/sdk.rs");

fn main() {
    serve(
        "policy.propose_prompt",
        "tool",
        &mut |method, params| match method {
            "policy.propose_prompt" => {
                let dir = match std::env::var("HS_RUN_DIR") {
                    Ok(d) => d,
                    Err(_) => return serde_json::json!({"$error": "HS_RUN_DIR not set"}),
                };
                let name = params["name"].as_str().unwrap_or("swe-mission");
                let text = params["text"].as_str().unwrap_or("");
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
