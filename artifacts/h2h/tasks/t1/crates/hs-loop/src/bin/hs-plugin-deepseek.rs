//! Real-model plugin "deepseek": `DeepSeek` via api.deepseek.com (OpenAI-shaped).
//! Key from `HS_DEEPSEEK_API_KEY` or `HS_DEEPSEEK_API_KEY_FILE`.
//! Used by the real-model ablation re-run; the unit suite stays on the
//! scripted benchmodel.
include!("shared/sdk.rs");

fn main() {
    // Preflight (first-run gate, stranger burn 2026-09-10): credential
    // presence checked at session load; the error names the exact fix.
    register_preflight(|| {
        hs_loop::realmodel::load_key(&hs_loop::realmodel::deepseek()).map(|_| ())
    });
    serve_ext("deepseek", "model", &mut |method, params, emit| match method {
        "model.call" => {
            let tools = params.get("tools");
            let stream = params["stream_deltas"].as_bool() == Some(true);
            let r = if let Some(msgs) = params.get("messages") {
                if stream {
                    hs_loop::realmodel::call_messages_streaming(
                        &hs_loop::realmodel::deepseek(),
                        msgs,
                        tools,
                        &|d| emit(serde_json::json!({"delta": d})),
                    )
                } else {
                    hs_loop::realmodel::call_messages(&hs_loop::realmodel::deepseek(), msgs, tools)
                }
            } else if stream {
                hs_loop::realmodel::call_streaming(
                    &hs_loop::realmodel::deepseek(),
                    params["prompt"].as_str().unwrap_or(""),
                    tools,
                    &|d| emit(serde_json::json!({"delta": d})),
                )
            } else {
                hs_loop::realmodel::call(
                    &hs_loop::realmodel::deepseek(),
                    params["prompt"].as_str().unwrap_or(""),
                    tools,
                )
            };
            match r {
                Ok(v) => v,
                Err(e) => serde_json::json!({"$error": e}),
            }
        }
        _ => serde_json::json!({"$error": "unknown method"}),
    });
}
