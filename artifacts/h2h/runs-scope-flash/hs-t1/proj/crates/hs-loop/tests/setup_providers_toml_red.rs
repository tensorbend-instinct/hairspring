//! RED contract tests: the guided setup + readiness gate must see
//! TOML-declared providers, not only the builtins (Eric 2026-09-11:
//! "I downloaded the latest but don't see how to use openrouter" - the
//! wizard offering only deepseek/glm is the discoverability gap).

use std::io::Write;

static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

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

fn toml_file(body: &str) -> tempfile::NamedTempFile {
    let mut f = tempfile::NamedTempFile::new().unwrap();
    f.write_all(body.as_bytes()).unwrap();
    f
}

#[test]
fn readiness_gate_lists_toml_providers() {
    let _g = ENV_LOCK.lock().unwrap();
    let f = toml_file(OPENROUTER);
    unsafe { std::env::set_var("HS_PROVIDERS_TOML", f.path()) };
    let rows = hs_loop::setup::check_readiness();
    unsafe { std::env::remove_var("HS_PROVIDERS_TOML") };
    let names: Vec<&str> = rows.iter().map(|r| r.provider.as_str()).collect();
    assert!(names.contains(&"deepseek"), "builtin lost: {names:?}");
    assert!(names.contains(&"glm"), "builtin lost: {names:?}");
    assert!(
        names.contains(&"openrouter"),
        "TOML provider invisible to the wizard: {names:?}"
    );
}

#[test]
fn toml_provider_does_not_duplicate_a_builtin() {
    let _g = ENV_LOCK.lock().unwrap();
    let f = toml_file(
        "[[providers]]\nname = \"glm\"\nbase_url = \"https://x/chat/completions\"\nmodel = \"m\"\n",
    );
    unsafe { std::env::set_var("HS_PROVIDERS_TOML", f.path()) };
    let rows = hs_loop::setup::check_readiness();
    unsafe { std::env::remove_var("HS_PROVIDERS_TOML") };
    let n = rows.iter().filter(|r| r.provider == "glm").count();
    assert_eq!(n, 1, "glm listed {n} times");
}

#[test]
fn save_key_accepts_a_toml_provider() {
    let _g = ENV_LOCK.lock().unwrap();
    let f = toml_file(OPENROUTER);
    let home = tempfile::tempdir().unwrap();
    unsafe {
        std::env::set_var("HS_PROVIDERS_TOML", f.path());
        std::env::set_var("XDG_CONFIG_HOME", home.path());
    }
    let r = hs_loop::setup::save_key("openrouter", "sk-or-test");
    unsafe {
        std::env::remove_var("HS_PROVIDERS_TOML");
        std::env::remove_var("XDG_CONFIG_HOME");
    }
    let path = r.expect("save_key rejected a TOML provider");
    assert_eq!(
        std::fs::read_to_string(&path).unwrap().trim(),
        "sk-or-test"
    );
    assert!(path.ends_with("keys/openrouter.key"));
}
