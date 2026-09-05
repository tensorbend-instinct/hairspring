//! Generic OpenAI-compatible model plugin: providers are configuration.
//! Env: HS_PROVIDERS_TOML (path to a [[providers]] TOML), HS_PROVIDER (name
//! in that file). Falls back to the builtin glm/deepseek tables when no TOML
//! is given, so the retired hs-plugin-glm/deepseek shims keep working until
//! their gate closes. Keys stay fill-only (env indirection, vault-populated).
include!("shared/sdk.rs");

fn main() {
    let provider = std::env::var("HS_PROVIDER").unwrap_or_else(|_| "glm".into());
    let p = match std::env::var("HS_PROVIDERS_TOML") {
        Ok(path) => {
            let cfgs = hs_loop::realmodel::load_providers_toml(std::path::Path::new(&path))
                .unwrap_or_else(|e| {
                    eprintln!("hs-plugin-model: {e}");
                    std::process::exit(2);
                });
            hs_loop::realmodel::provider_from_config(
                hs_loop::realmodel::find_provider(&cfgs, &provider)
                    .unwrap_or_else(|e| {
                        eprintln!("hs-plugin-model: {e}");
                        std::process::exit(2);
                    }),
            )
            .unwrap_or_else(|e| {
                eprintln!("hs-plugin-model: {e}");
                std::process::exit(2);
            })
        }
        Err(_) => match provider.as_str() {
            "glm" => hs_loop::realmodel::glm(),
            "deepseek" => hs_loop::realmodel::deepseek(),
            other => {
                eprintln!("hs-plugin-model: unknown builtin provider '{other}' (set HS_PROVIDERS_TOML)");
                std::process::exit(2);
            }
        },
    };
    serve("model", "model", &mut move |method, params| match method {
        "model.call" => {
            let prompt = params["prompt"].as_str().unwrap_or("");
            let tools = params.get("tools");
            match hs_loop::realmodel::call(&p, prompt, tools) {
                Ok(v) => v,
                Err(e) => serde_json::json!({"$error": e}),
            }
        }
        _ => serde_json::json!({"$error": "unknown method"}),
    });
}
