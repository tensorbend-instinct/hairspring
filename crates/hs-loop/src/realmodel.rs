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
    pub name: &'static str,
    pub base_url_env: &'static str,
    pub default_base_url: &'static str,
    pub model_env: &'static str,
    pub default_model: &'static str,
    pub key_env: &'static str,
    pub key_file_env: &'static str,
    pub price_in_env: &'static str,
    pub default_in_micros: f64,
    pub price_cached_env: &'static str,
    pub default_cached_micros: f64,
    pub price_out_env: &'static str,
    pub default_out_micros: f64,
    /// Optional env var holding a JSON object merged into the request body
    /// (e.g. HS_DEEPSEEK_EXTRA_BODY_JSON='{"thinking":{"type":"disabled"}}').
    pub extra_body_json_env: &'static str,
}

pub const GLM: Provider = Provider {
    name: "glm",
    base_url_env: "HS_GLM_BASE_URL",
    default_base_url: "https://api.z.ai/api/paas/v4/chat/completions",
    model_env: "HS_GLM_MODEL",
    default_model: "glm-5.3",
    key_env: "HS_GLM_API_KEY",
    key_file_env: "HS_GLM_API_KEY_FILE",
    price_in_env: "HS_GLM_PRICE_IN_MICROS",
    default_in_micros: 1.40,
    price_cached_env: "HS_GLM_PRICE_CACHED_MICROS",
    default_cached_micros: 0.26,
    price_out_env: "HS_GLM_PRICE_OUT_MICROS",
    default_out_micros: 4.40,
    extra_body_json_env: "HS_GLM_EXTRA_BODY_JSON",
};

pub const DEEPSEEK: Provider = Provider {
    name: "deepseek",
    base_url_env: "HS_DEEPSEEK_BASE_URL",
    default_base_url: "https://api.deepseek.com/chat/completions",
    model_env: "HS_DEEPSEEK_MODEL",
    default_model: "deepseek-v4-flash",
    key_env: "HS_DEEPSEEK_API_KEY",
    key_file_env: "HS_DEEPSEEK_API_KEY_FILE",
    price_in_env: "HS_DEEPSEEK_PRICE_IN_MICROS",
    default_in_micros: 0.44,
    price_cached_env: "HS_DEEPSEEK_PRICE_CACHED_MICROS",
    default_cached_micros: 0.014,
    price_out_env: "HS_DEEPSEEK_PRICE_OUT_MICROS",
    default_out_micros: 1.32,
    extra_body_json_env: "HS_DEEPSEEK_EXTRA_BODY_JSON",
};

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
    if let Ok(k) = std::env::var(p.key_env) {
        if !k.trim().is_empty() {
            return Ok(k.trim().to_string());
        }
    }
    if let Ok(path) = std::env::var(p.key_file_env) {
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

pub fn call(p: &Provider, prompt: &str) -> Result<serde_json::Value, String> {
    let key = load_key(p)?;
    let url = env_or(p.base_url_env, p.default_base_url);
    let model = env_or(p.model_env, p.default_model);
    let mut body = json!({
        "model": model,
        "temperature": 0,
        "messages": [
            {"role": "system", "content": SYSTEM},
            {"role": "user", "content": prompt},
        ],
    });
    if let Ok(extra) = std::env::var(p.extra_body_json_env) {
        let extra: serde_json::Value = serde_json::from_str(&extra)
            .map_err(|e| format!("{}: bad {}: {e}", p.name, p.extra_body_json_env))?;
        if let (Some(b), Some(x)) = (body.as_object_mut(), extra.as_object()) {
            for (k, v) in x {
                b.insert(k.clone(), v.clone());
            }
        }
    }
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(120)))
        .build()
        .into();
    let resp = agent
        .post(&url)
        .header("Authorization", &format!("Bearer {key}"))
        .header("Content-Type", "application/json")
        .send_json(&body);
    let mut resp = match resp {
        Ok(r) => r,
        Err(e) => return Err(format!("{}: request failed: {e}", p.name)), // ureq errors never echo headers
    };
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
    let raw = v["choices"][0]["message"]["content"]
        .as_str()
        .ok_or_else(|| format!("{}: no choices[0].message.content", p.name))?;
    let completion = extract_json_object(raw)
        .ok_or_else(|| format!("{}: no JSON object in model output", p.name))?
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
    let pin = env_f64(p.price_in_env, p.default_in_micros);
    let pcached = env_f64(p.price_cached_env, p.default_cached_micros);
    let pout = env_f64(p.price_out_env, p.default_out_micros);
    let cost = (cached as f64 * pcached
        + (input_tokens - cached) as f64 * pin
        + output_tokens as f64 * pout)
        .round() as i64;
    let r = CallResult {
        completion,
        input_tokens,
        output_tokens,
        cached_tokens: cached,
        cost_usd_micros: cost,
    };
    Ok(json!({
        "completion": r.completion,
        "input_tokens": r.input_tokens,
        "output_tokens": r.output_tokens,
        "cached_tokens": r.cached_tokens,
        "cost_usd_micros": r.cost_usd_micros,
        "provider_model": model,
    }))
}
