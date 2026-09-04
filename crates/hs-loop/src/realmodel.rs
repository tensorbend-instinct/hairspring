//! Real-model adapters (GLM via z.ai, DeepSeek via api.deepseek.com).
//!
//! Both providers expose an OpenAI-shaped chat-completions endpoint with
//! Bearer auth. The API key comes ONLY from an env var or a 0600 file
//! outside the repo (populated via vault fill at run time) - it is never
//! logged, never in fixtures, never committed. Prices are micro-USD per
//! token (numerically equal to USD per 1M tokens), grounded 2026-09-02:
//!   GLM-5.3: in $1.40/M, cached $0.26/M, out $4.40/M  (docs.z.ai/guides/overview/pricing)
//!   deepseek-v4-flash (= "deepseek-chat" alias = base V4 latest, verified
//!   2026-09-02): peak in $0.44/M (miss), $0.014/M (hit), out $1.32/M;
//!   off-peak half  (api-docs.deepseek.com/quick_start/pricing)
//! Rates and endpoints are env-overridable so a price change is a config
//! change, not a code change.

use serde_json::json;
use std::time::Duration;

#[derive(Clone, Debug)]
pub struct Provider {
    pub name: String,
    pub base_url_env: String,
    pub default_base_url: String,
    pub model_env: String,
    pub default_model: String,
    pub key_env: String,
    pub key_file_env: String,
    pub price_in_env: String,
    pub default_in_micros: f64,
    pub price_cached_env: String,
    pub default_cached_micros: f64,
    pub price_out_env: String,
    pub default_out_micros: f64,
    /// Optional env var holding a JSON object merged into the request body
    /// (e.g. HS_DEEPSEEK_EXTRA_BODY_JSON='{"thinking":{"type":"disabled"}}').
    pub extra_body_json_env: String,
    /// TOML-sourced extra body; the env var above wins when both are set.
    pub default_extra_body_json: Option<String>,
}

fn builtin(
    name: &str,
    default_base_url: &str,
    default_model: &str,
    in_micros: f64,
    cached_micros: f64,
    out_micros: f64,
    default_extra: Option<&str>,
) -> Provider {
    let up = name.to_uppercase().replace('-', "_");
    Provider {
        name: name.into(),
        base_url_env: format!("HS_{up}_BASE_URL"),
        default_base_url: default_base_url.into(),
        model_env: format!("HS_{up}_MODEL"),
        default_model: default_model.into(),
        key_env: format!("HS_{up}_API_KEY"),
        key_file_env: format!("HS_{up}_API_KEY_FILE"),
        price_in_env: format!("HS_{up}_PRICE_IN_MICROS"),
        default_in_micros: in_micros,
        price_cached_env: format!("HS_{up}_PRICE_CACHED_MICROS"),
        default_cached_micros: cached_micros,
        price_out_env: format!("HS_{up}_PRICE_OUT_MICROS"),
        default_out_micros: out_micros,
        extra_body_json_env: format!("HS_{up}_EXTRA_BODY_JSON"),
        default_extra_body_json: default_extra.map(|s| s.to_string()),
    }
}

pub fn glm() -> Provider {
    builtin(
        "glm",
        "https://api.z.ai/api/paas/v4/chat/completions",
        "glm-5.3",
        1.40,
        0.26,
        4.40,
        None,
    )
}

pub fn deepseek() -> Provider {
    builtin(
        "deepseek",
        "https://api.deepseek.com/chat/completions",
        "deepseek-v4-flash",
        0.44,
        0.014,
        1.32,
        None,
    )
}

/// One [[providers]] entry. Key material never appears here: key_env NAMES
/// the env var that holds it (vault-populated, fill-only).
#[derive(Clone, Debug, serde::Deserialize)]
pub struct ProviderConfig {
    pub name: String,
    pub base_url: String,
    pub model: String,
    pub key_env: Option<String>,
    pub extra_body_json: Option<String>,
    pub price_in_micros: Option<f64>,
    pub price_cached_micros: Option<f64>,
    pub price_out_micros: Option<f64>,
}

#[derive(serde::Deserialize)]
struct ProvidersFile {
    providers: Vec<ProviderConfig>,
}

pub fn load_providers_toml(path: &std::path::Path) -> Result<Vec<ProviderConfig>, String> {
    let text = std::fs::read_to_string(path)
        .map_err(|e| format!("providers TOML {}: {e}", path.display()))?;
    let f: ProvidersFile =
        toml::from_str(&text).map_err(|e| format!("providers TOML {}: {e}", path.display()))?;
    if f.providers.is_empty() {
        return Err(format!("providers TOML {}: no [[providers]] entries", path.display()));
    }
    Ok(f.providers)
}

pub fn find_provider<'a>(cfgs: &'a [ProviderConfig], name: &str) -> Result<&'a ProviderConfig, String> {
    cfgs.iter()
        .find(|c| c.name == name)
        .ok_or_else(|| format!("unknown provider '{name}' (not in providers TOML)"))
}

/// Resolve a TOML entry into a working Provider. Env overrides still win:
/// HS_{NAME}_BASE_URL over the TOML base_url, and so on.
pub fn provider_from_config(c: &ProviderConfig) -> Result<Provider, String> {
    let up = c.name.to_uppercase().replace('-', "_");
    let base_url_env = format!("HS_{up}_BASE_URL");
    let default_base_url = env_or(&base_url_env, &c.base_url);
    Ok(Provider {
        name: c.name.clone(),
        base_url_env: String::new(), // already resolved above
        default_base_url,
        model_env: format!("HS_{up}_MODEL"),
        default_model: c.model.clone(),
        key_env: c.key_env.clone().unwrap_or_else(|| format!("HS_{up}_API_KEY")),
        key_file_env: format!("HS_{up}_API_KEY_FILE"),
        price_in_env: format!("HS_{up}_PRICE_IN_MICROS"),
        default_in_micros: c.price_in_micros.unwrap_or(0.0),
        price_cached_env: format!("HS_{up}_PRICE_CACHED_MICROS"),
        default_cached_micros: c.price_cached_micros.unwrap_or(0.0),
        price_out_env: format!("HS_{up}_PRICE_OUT_MICROS"),
        default_out_micros: c.price_out_micros.unwrap_or(0.0),
        extra_body_json_env: format!("HS_{up}_EXTRA_BODY_JSON"),
        default_extra_body_json: c.extra_body_json.clone(),
    })
}

const SYSTEM: &str = "You are the model plugin of an autonomous coding agent. \
Reply with EXACTLY one JSON object and nothing else (no markdown fences, no prose): \
{\"tool\":\"answer.write\",\"args\":{\"path\":<the ANSWER_PATH value from the prompt>,\
\"content\":<your best answer as a string>}}. If FEEDBACK names an expected token, \
make the content exactly that token.";

fn env_or(key: &str, default: &str) -> String {
    std::env::var(key)
        .ok()
        .filter(|v| !v.is_empty())
        .unwrap_or_else(|| default.to_string())
}

fn env_f64(key: &str, default: f64) -> f64 {
    std::env::var(key)
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}

/// Load the API key from env or file. The value is returned for the auth
/// header and must never be logged or included in an error.
pub fn load_key(p: &Provider) -> Result<String, String> {
    if let Ok(k) = std::env::var(&p.key_env) {
        if !k.trim().is_empty() {
            return Ok(k.trim().to_string());
        }
    }
    if let Ok(path) = std::env::var(&p.key_file_env) {
        return std::fs::read_to_string(&path)
            .map(|s| s.trim().to_string())
            .map_err(|e| format!("{}: cannot read key file {path}: {e}", p.name));
    }
    Err(format!(
        "{}: no API key - set {} or {} (vault-populated, never in the repo)",
        p.name, p.key_env, p.key_file_env
    ))
}

/// Extract the first balanced JSON object from model output that may be
/// wrapped in markdown fences or prose.
pub fn extract_json_object(s: &str) -> Option<&str> {
    let start = s.find('{')?;
    let mut depth = 0i32;
    let mut in_str = false;
    let mut esc = false;
    for (i, c) in s.char_indices().skip_while(|(i, _)| *i < start) {
        if in_str {
            if esc {
                esc = false;
            } else if c == '\\' {
                esc = true;
            } else if c == '"' {
                in_str = false;
            }
            continue;
        }
        match c {
            '"' => in_str = true,
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    return Some(&s[start..=i]);
                }
            }
            _ => {}
        }
    }
    None
}

pub struct CallResult {
    pub completion: String,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cached_tokens: u64,
    pub cost_usd_micros: i64,
}

fn attempt(
    p: &Provider,
    agent: &ureq::Agent,
    url: &str,
    key: &str,
    body: &serde_json::Value,
    model: &str,
) -> Result<serde_json::Value, String> {
    let mut resp = agent
        .post(url)
        .header("Authorization", &format!("Bearer {key}"))
        .header("Content-Type", "application/json")
        .send_json(body)
        .map_err(|e| format!("{}: request failed: {e}", p.name))?;
    let status = resp.status();
    if !status.is_success() {
        return Err(format!(
            "{}: HTTP {} from provider",
            p.name,
            status.as_u16()
        ));
    }
    let v: serde_json::Value = resp
        .body_mut()
        .read_json()
        .map_err(|e| format!("{}: unparsable provider response: {e}", p.name))?;
    let fr = v["choices"][0]["finish_reason"]
        .as_str()
        .unwrap_or("<none>")
        .to_string();
    let raw = v["choices"][0]["message"]["content"]
        .as_str()
        .ok_or_else(|| format!("{}: no choices[0].message.content (finish_reason={})", p.name, fr))?;
    let completion = extract_json_object(raw)
        .ok_or_else(|| format!(
            "{}: no JSON object in model output (finish_reason={}, content_len={}, head={:.80})",
            p.name, fr, raw.len(), raw))?
        .to_string();
    let usage = &v["usage"];
    let input_tokens = usage["prompt_tokens"].as_u64().unwrap_or(0);
    let output_tokens = usage["completion_tokens"].as_u64().unwrap_or(0);
    // deepseek: prompt_cache_hit_tokens; openai-style: prompt_tokens_details.cached_tokens
    let cached = usage["prompt_cache_hit_tokens"]
        .as_u64()
        .or_else(|| usage["prompt_tokens_details"]["cached_tokens"].as_u64())
        .unwrap_or(0)
        .min(input_tokens);
    let pin = env_f64(&p.price_in_env, p.default_in_micros);
    let pcached = env_f64(&p.price_cached_env, p.default_cached_micros);
    let pout = env_f64(&p.price_out_env, p.default_out_micros);
    let cost = (cached as f64 * pcached
        + (input_tokens - cached) as f64 * pin
        + output_tokens as f64 * pout)
        .round() as i64;
    Ok(json!({
        "completion": completion,
        "input_tokens": input_tokens,
        "output_tokens": output_tokens,
        "cached_tokens": cached,
        "cost_usd_micros": cost,
        "provider_model": model,
    }))
}

/// Sentinel completion returned when the provider holds a call past the
/// watchdog. It is deliberately NOT a JSON tool call: the inner loop records
/// it as a malformed completion and feeds it back, so a hung provider costs
/// the mission a step instead of hanging the harness forever.
pub const WATCHDOG_SENTINEL: &str = "__provider_watchdog_timeout__";

/// Watchdog per attempt (seconds), env-overridable. Re-grounded 2026-09-03:
/// measured max-effort thinking calls run 80s (convergent context) to 270s+
/// (non-convergent, 13.7k reasoning tokens); low-effort ~18s. 420s default;
/// the realbench run used 900s via HS_REALMODEL_CALL_TIMEOUT_SECS.
/// ureq's timeout_global (600s) demonstrably does NOT fire on a stalled
/// response-body read (observed: calls stuck 31+ min, zero harness events),
/// so the watchdog wraps the entire attempt in a thread with a recv_timeout.
pub fn watchdog_secs() -> u64 {
    std::env::var("HS_REALMODEL_CALL_TIMEOUT_SECS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(420)
}

pub fn call(p: &Provider, prompt: &str) -> Result<serde_json::Value, String> {
    let key = load_key(p)?;
    let url = env_or(&p.base_url_env, &p.default_base_url);
    let model = env_or(&p.model_env, &p.default_model);
    let mut body = json!({
        "model": model,
        "temperature": 0,
        "max_tokens": 32768,
        "messages": [
            {"role": "system", "content": SYSTEM},
            {"role": "user", "content": prompt},
        ],
    });
    let extra_src = std::env::var(&p.extra_body_json_env)
        .ok()
        .or_else(|| p.default_extra_body_json.clone());
    if let Some(extra) = extra_src {
        let extra: serde_json::Value = serde_json::from_str(&extra)
            .map_err(|e| format!("{}: bad extra_body_json: {e}", p.name))?;
        if let (Some(b), Some(x)) = (body.as_object_mut(), extra.as_object()) {
            for (k, v) in x {
                b.insert(k.clone(), v.clone());
            }
        }
    }
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(1500)))
        .build()
        .into();
    let watchdog = watchdog_secs();
    let mut last_err = String::new();
    for attempt_no in 0..3 {
        if attempt_no > 0 {
            std::thread::sleep(Duration::from_secs(5 * attempt_no as u64));
        }
        let (tx, rx) = std::sync::mpsc::channel();
        let (a, u, k, b, m) = (
            agent.clone(),
            url.clone(),
            key.clone(),
            body.clone(),
            model.clone(),
        );
        let pt = p.clone();
        std::thread::spawn(move || {
            let r = attempt(&pt, &a, &u, &k, &b, &m);
            let _ = tx.send(r);
        });
        match rx.recv_timeout(Duration::from_secs(watchdog)) {
            Ok(Ok(v)) => return Ok(v),
            Ok(Err(e)) => {
                eprintln!("realmodel {} attempt {} failed: {}", p.name, attempt_no + 1, e);
                last_err = e;
            }
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                // Hung provider: do NOT retry (a hung endpoint hangs retries
                // too). Sentinel = feedback, not a harness error.
                return Ok(json!({
                    "completion": WATCHDOG_SENTINEL,
                    "input_tokens": 0,
                    "output_tokens": 0,
                    "cached_tokens": 0,
                    "cost_usd_micros": 0,
                    "provider_model": model,
                }));
            }
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                last_err = format!("{}: worker thread died", p.name);
            }
        }
    }
    Err(last_err)
}
