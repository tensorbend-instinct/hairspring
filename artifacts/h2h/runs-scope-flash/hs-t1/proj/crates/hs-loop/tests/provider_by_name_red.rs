//! RED contract tests: one resolution path for every provider - builtins
//! (deepseek, glm) plus any [[providers]] TOML entry (OpenAI-compatible:
//! OpenAI direct, OpenRouter, ...). Adding a provider is a TOML entry, not
//! new code. The critic resolves models through the SAME path so
//! HS_CRITIC_MODEL names any provider.

use hs_loop::realmodel::*;
use std::io::Write;

static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn toml_file(body: &str) -> tempfile::NamedTempFile {
    let mut f = tempfile::NamedTempFile::new().unwrap();
    f.write_all(body.as_bytes()).unwrap();
    f
}

const OPENROUTER: &str = r#"
[[providers]]
name = "openrouter"
base_url = "https://openrouter.ai/api/v1/chat/completions"
model = "openai/gpt-6-astra-pro"
key_env = "HS_OPENROUTER_API_KEY"
price_in_micros = 10.0
price_cached_micros = 1.0
price_out_micros = 50.0
"#;

#[test]
fn builtin_names_still_resolve() {
    let g = provider_by_name("glm").unwrap();
    assert_eq!(g.default_base_url, glm().default_base_url);
    let d = provider_by_name("deepseek").unwrap();
    assert_eq!(d.default_model, deepseek().default_model);
}

#[test]
fn toml_fallback_resolves_openrouter() {
    let _g = ENV_LOCK.lock().unwrap();
    let f = toml_file(OPENROUTER);
    let prev = std::env::var("HS_PROVIDERS_TOML").ok();
    unsafe { std::env::set_var("HS_PROVIDERS_TOML", f.path()) };
    let r = provider_by_name("openrouter");
    match prev {
        Some(v) => unsafe { std::env::set_var("HS_PROVIDERS_TOML", v) },
        None => unsafe { std::env::remove_var("HS_PROVIDERS_TOML") },
    }
    let p = r.unwrap();
    assert_eq!(p.default_base_url, "https://openrouter.ai/api/v1/chat/completions");
    assert_eq!(p.default_model, "openai/gpt-6-astra-pro");
    assert_eq!(p.key_env, "HS_OPENROUTER_API_KEY");
    assert_eq!(p.default_out_micros, 50.0);
}

#[test]
fn unknown_provider_error_names_it() {
    let _g = ENV_LOCK.lock().unwrap();
    let prev = std::env::var("HS_PROVIDERS_TOML").ok();
    unsafe { std::env::remove_var("HS_PROVIDERS_TOML") };
    let e = provider_by_name("bogus").unwrap_err();
    if let Some(v) = prev {
        unsafe { std::env::set_var("HS_PROVIDERS_TOML", v) };
    }
    assert!(e.contains("bogus"), "{e}");
}

#[test]
fn critic_resolves_through_the_same_path() {
    let _g = ENV_LOCK.lock().unwrap();
    let f = toml_file(OPENROUTER);
    let prev = std::env::var("HS_PROVIDERS_TOML").ok();
    unsafe { std::env::set_var("HS_PROVIDERS_TOML", f.path()) };
    let r = hs_loop::critic::provider_for("openrouter");
    match prev {
        Some(v) => unsafe { std::env::set_var("HS_PROVIDERS_TOML", v) },
        None => unsafe { std::env::remove_var("HS_PROVIDERS_TOML") },
    }
    let p = r.unwrap();
    assert_eq!(p.name, "openrouter");
    // builtins unchanged
    assert!(hs_loop::critic::provider_for("deepseek").is_ok());
    assert!(hs_loop::critic::provider_for("glm").is_ok());
}
