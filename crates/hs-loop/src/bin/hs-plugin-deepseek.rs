//! Real-model plugin "deepseek": DeepSeek via api.deepseek.com (OpenAI-shaped).
//! Key from HS_GLM_API_KEY or HS_GLM_API_KEY_FILE (vault-populated).
//! Used by the real-model ablation re-run; the unit suite stays on the
//! scripted benchmodel.
include!("shared/sdk.rs");

fn main() {
    serve("deepseek", "model", &mut |method, params| match method {
        "model.call" => {
            let prompt = params["prompt"].as_str().unwrap_or("");
            match hs_loop::realmodel::call(&hs_loop::realmodel::DEEPSEEK, prompt) {
                Ok(v) => v,
                Err(e) => serde_json::json!({"$error": e}),
            }
        }
        _ => serde_json::json!({"$error": "unknown method"}),
    });
}
