//! Tool "web.search": Brave Search API. Key: HS_BRAVE_API_KEY or
//! HS_BRAVE_API_KEY_FILE. Endpoint override HS_SEARCH_URL (tests).
include!("shared/sdk.rs");

fn key() -> Option<String> {
    if let Ok(k) = std::env::var("HS_BRAVE_API_KEY") {
        if !k.trim().is_empty() {
            return Some(k.trim().to_string());
        }
    }
    let f = std::env::var("HS_BRAVE_API_KEY_FILE").ok()?;
    let k = std::fs::read_to_string(f).ok()?;
    let k = k.trim();
    (!k.is_empty()).then(|| k.to_string())
}

fn strip_tags(s: &str) -> String {
    let mut out = String::new();
    let mut in_tag = false;
    for c in s.chars() {
        match c {
            '<' => in_tag = true,
            '>' => in_tag = false,
            _ if !in_tag => out.push(c),
            _ => {}
        }
    }
    out.replace("&amp;", "&").replace("&quot;", "\"").replace("&#x27;", "'").trim().to_string()
}

fn main() {
    serve("web.search", "tool", &mut |method, params| match method {
        "tool.call" => {
            let Some(q) = params["args"]["query"].as_str().filter(|s| !s.trim().is_empty()) else {
                return serde_json::json!({"$error": "query is required"});
            };
            let Some(k) = key() else {
                return serde_json::json!({"$error": "web.search needs a Brave Search API key: set HS_BRAVE_API_KEY or HS_BRAVE_API_KEY_FILE (free tier at api.search.brave.com)"});
            };
            let count = params["args"]["count"].as_u64().unwrap_or(5).clamp(1, 20);
            let url = std::env::var("HS_SEARCH_URL").unwrap_or_else(|_| "https://api.search.brave.com/res/v1/web/search".into());
            let agent: ureq::Agent = ureq::Agent::config_builder()
                .timeout_global(Some(std::time::Duration::from_secs(20)))
                .http_status_as_error(false)
                .build()
                .into();
            let resp = agent
                .get(&url)
                .query("q", q)
                .query("count", count.to_string())
                .header("X-Subscription-Token", &k)
                .header("Accept", "application/json")
                .call();
            let mut resp = match resp {
                Ok(r) => r,
                Err(e) => return serde_json::json!({"$error": format!("search failed: {e}")}),
            };
            let status = resp.status().as_u16();
            let v: serde_json::Value = match resp.body_mut().read_json() {
                Ok(v) => v,
                Err(e) => return serde_json::json!({"$error": format!("search reply unreadable (HTTP {status}): {e}")}),
            };
            if status >= 400 {
                return serde_json::json!({"$error": format!("search HTTP {status}")});
            }
            let results: Vec<serde_json::Value> = v["web"]["results"]
                .as_array()
                .map(|a| {
                    a.iter()
                        .map(|r| serde_json::json!({
                            "title": strip_tags(r["title"].as_str().unwrap_or("")),
                            "url": r["url"],
                            "snippet": strip_tags(r["description"].as_str().unwrap_or(""))}))
                        .collect()
                })
                .unwrap_or_default();
            serde_json::json!({"ok": true, "query": q, "results": results})
        }
        _ => serde_json::json!({"$error": "unknown method"}),
    });
}
