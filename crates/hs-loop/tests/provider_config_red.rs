//! RED contract tests: providers are configuration (Eric's ruling, spec TOML
//! intent). One generic OpenAI-compatible plugin; adding a provider is a TOML
//! entry, not new code. Key material stays fill-only via env indirection -
//! TOML names the env var, never holds the key.

use hs_loop::realmodel::*;
use std::io::Write;

static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn toml_file(body: &str) -> tempfile::NamedTempFile {
    let mut f = tempfile::NamedTempFile::new().unwrap();
    f.write_all(body.as_bytes()).unwrap();
    f
}

const TWO_PROVIDERS: &str = r#"
[[providers]]
name = "glm"
base_url = "https://api.z.ai/api/coding/paas/v4/chat/completions"
model = "glm-5.3"
key_env = "HS_GLM_API_KEY"
extra_body_json = '{"reasoning_effort":"max"}'
price_in_micros = 1.40
price_cached_micros = 0.26
price_out_micros = 4.40

[[providers]]
name = "deepseek"
base_url = "https://api.deepseek.com/chat/completions"
model = "deepseek-v4-flash"
key_env = "HS_DEEPSEEK_API_KEY"
price_in_micros = 0.44
price_cached_micros = 0.014
price_out_micros = 1.32
"#;

#[test]
fn loads_providers_from_toml() {
    let f = toml_file(TWO_PROVIDERS);
    let cfgs = load_providers_toml(f.path()).unwrap();
    assert_eq!(cfgs.len(), 2);
    assert_eq!(cfgs[0].name, "glm");
    assert_eq!(cfgs[0].model, "glm-5.3");
    assert_eq!(
        cfgs[0].extra_body_json.as_deref(),
        Some("{\"reasoning_effort\":\"max\"}")
    );
    assert_eq!(cfgs[1].name, "deepseek");
    assert!(cfgs[1].extra_body_json.is_none());
}

#[test]
fn malformed_toml_is_an_error_not_a_panic() {
    let f = toml_file("[[providers]\nname = 42\n");
    assert!(load_providers_toml(f.path()).is_err());
    assert!(load_providers_toml(std::path::Path::new("/nonexistent/x.toml")).is_err());
}

#[test]
fn lookup_by_name_and_unknown_name_errors() {
    let f = toml_file(TWO_PROVIDERS);
    let cfgs = load_providers_toml(f.path()).unwrap();
    let c = find_provider(&cfgs, "deepseek").unwrap();
    assert_eq!(c.model, "deepseek-v4-flash");
    let e = find_provider(&cfgs, "nonexistent").unwrap_err();
    assert!(e.contains("nonexistent"), "error names the provider: {e}");
}

#[test]
fn toml_provider_becomes_working_provider_and_env_wins() {
    let _g = ENV_LOCK.lock().unwrap();
    let f = toml_file(TWO_PROVIDERS);
    let cfgs = load_providers_toml(f.path()).unwrap();
    let glm_cfg = find_provider(&cfgs, "glm").unwrap();
    let p = provider_from_config(glm_cfg).unwrap();
    // TOML values land on the provider
    assert_eq!(p.name, "glm");
    assert_eq!(p.default_model, "glm-5.3");
    assert_eq!(
        p.default_base_url,
        "https://api.z.ai/api/coding/paas/v4/chat/completions"
    );
    // env override still beats the TOML value (ops override without a redeploy)
    std::env::set_var("HS_GLM_BASE_URL", "http://127.0.0.1:1/override");
    let p2 = provider_from_config(glm_cfg).unwrap();
    assert_eq!(p2.default_base_url, "http://127.0.0.1:1/override");
    std::env::remove_var("HS_GLM_BASE_URL");
}

#[test]
fn missing_key_is_fill_only_error() {
    let _g = ENV_LOCK.lock().unwrap();
    let f = toml_file(TWO_PROVIDERS);
    let cfgs = load_providers_toml(f.path()).unwrap();
    let glm_cfg = find_provider(&cfgs, "glm").unwrap();
    std::env::remove_var("HS_GLM_API_KEY");
    std::env::remove_var("HS_GLM_API_KEY_FILE");
    let e = provider_from_config(glm_cfg)
        .and_then(|p| load_key(&p))
        .unwrap_err();
    assert!(e.contains("HS_GLM_API_KEY"), "names the env var: {e}");
    assert!(!e.to_lowercase().contains("secret"), "no key material: {e}");
}

#[test]
fn extra_body_json_from_toml_reaches_the_request() {
    let _g = ENV_LOCK.lock().unwrap();
    // minimal mock: capture request body
    use std::io::{BufRead, BufReader, Read};
    use std::net::TcpListener;
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let (tx, rx) = std::sync::mpsc::channel::<String>();
    std::thread::spawn(move || {
        let (stream, _) = listener.accept().unwrap();
        let mut reader = BufReader::new(stream);
        let mut len = 0usize;
        loop {
            let mut line = String::new();
            reader.read_line(&mut line).unwrap();
            let t = line.trim();
            if t.is_empty() {
                break;
            }
            if let Some(v) = t.to_ascii_lowercase().strip_prefix("content-length:") {
                len = v.trim().parse().unwrap();
            }
        }
        let mut body = vec![0u8; len];
        reader.read_exact(&mut body).unwrap();
        tx.send(String::from_utf8(body).unwrap()).unwrap();
        let resp_body = r#"{"choices":[{"message":{"content":"{\"tool\":\"answer.write\",\"args\":{\"path\":\"/p\",\"content\":\"ok\"}}"}}],"usage":{"prompt_tokens":10,"completion_tokens":5}}"#;
        let resp = format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}", resp_body.len(), resp_body);
        reader.get_mut().write_all(resp.as_bytes()).unwrap();
    });
    let toml = format!(
        "[[providers]]\nname = \"glm\"\nbase_url = \"http://127.0.0.1:{port}/chat/completions\"\nmodel = \"glm-5.3\"\nkey_env = \"HS_GLM_API_KEY\"\nextra_body_json = '{{\"reasoning_effort\":\"max\"}}'\nprice_in_micros = 1.40\nprice_out_micros = 4.40\n"
    );
    let f = toml_file(&toml);
    let cfgs = load_providers_toml(f.path()).unwrap();
    std::env::set_var("HS_GLM_API_KEY", "mock-key-fill-only");
    let p = provider_from_config(find_provider(&cfgs, "glm").unwrap()).unwrap();
    let out = call(&p, "MISSION: t\nANSWER_PATH: /p", None).unwrap();
    let comp: serde_json::Value =
        serde_json::from_str(out["completion"].as_str().unwrap()).unwrap();
    assert_eq!(comp["tool"], "answer.write");
    assert_eq!(comp["args"]["content"], "ok");
    let body = rx.recv_timeout(std::time::Duration::from_secs(5)).unwrap();
    let v: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(v["reasoning_effort"], "max", "extra body merged: {body}");
    std::env::remove_var("HS_GLM_API_KEY");
}

#[test]
fn builtin_glm_matches_toml_equivalent() {
    // The hardcoded table and its TOML twin must be interchangeable during
    // the shim period (one gate), then the builtins retire.
    let f = toml_file(TWO_PROVIDERS);
    let cfgs = load_providers_toml(f.path()).unwrap();
    let p = provider_from_config(find_provider(&cfgs, "glm").unwrap()).unwrap();
    assert_eq!(p.name, "glm");
    assert_eq!(p.default_in_micros, 1.40);
    assert_eq!(p.default_out_micros, 4.40);
}
