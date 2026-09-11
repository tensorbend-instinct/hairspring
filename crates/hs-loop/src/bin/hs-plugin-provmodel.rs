//! Generic OpenAI-compatible provider plugin (Eric: providers are
//! configuration). hs-plugin-provmodel <provider> resolves the provider by
//! name - builtin (deepseek, glm) or a [[providers]] TOML entry
//! (HS_PROVIDERS_TOML or ~/.config/hairspring/providers.toml) - and serves
//! model.call through the shared realmodel client. Key material stays
//! fill-only via the provider's key_env / conventional key file.
include!("shared/sdk.rs");

fn main() {
    let name = std::env::args()
        .nth(1)
        .unwrap_or_else(|| panic!("usage: hs-plugin-provmodel <provider>"));
    let prov = hs_loop::realmodel::provider_by_name(&name)
        .unwrap_or_else(|e| panic!("{e}"));
    let pre = prov.clone();
    register_preflight(move || hs_loop::realmodel::load_key(&pre).map(|_| ()));
    // The kernel requires the advertised name to match the [[models]]
    // entry: advertise the provider name itself.
    let label: &str = Box::leak(name.clone().into_boxed_str());
    serve(label, "model", &mut move |method, params| match method {
        "model.call" => {
            let tools = params.get("tools");
            let r = if let Some(msgs) = params.get("messages") {
                hs_loop::realmodel::call_messages(&prov, msgs, tools)
            } else {
                hs_loop::realmodel::call(
                    &prov,
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
