//! Tool "web.fetch": fetch an http(s) URL as text (dsh parity: web_fetch).
//! args: {url, max_bytes?}. Refuses non-http(s) schemes and private/loopback/
//! link-local targets (SSRF guard, redirects re-checked hop by hop) unless
//! HS_WEB_ALLOW_PRIVATE=1. Known limit: DNS is resolved for the check and again
//! by the client, so a rebinding host could differ between the two.
include!("shared/sdk.rs");
use std::io::Read;
use std::net::{IpAddr, ToSocketAddrs};

const DEFAULT_MAX: usize = 200_000;

fn is_private(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v) => {
            v.is_loopback() || v.is_private() || v.is_link_local() || v.is_unspecified()
                || v.is_broadcast() || v.octets()[0] == 0 || (v.octets()[0] == 100 && (64..128).contains(&v.octets()[1]))
        }
        IpAddr::V6(v) => {
            v.is_loopback() || v.is_unspecified() || (v.segments()[0] & 0xfe00) == 0xfc00
                || (v.segments()[0] & 0xffc0) == 0xfe80
                || v.to_ipv4_mapped().is_some_and(|m| is_private(IpAddr::V4(m)))
        }
    }
}

fn check(url: &str, allow_private: bool) -> Result<(), String> {
    let (scheme, rest) = url.split_once("://").ok_or("bad url: missing scheme")?;
    if scheme != "http" && scheme != "https" {
        return Err(format!("scheme '{scheme}' not allowed (http/https only)"));
    }
    if allow_private {
        return Ok(());
    }
    let auth = rest.split(['/', '?', '#']).next().unwrap_or("");
    let hostport = auth.rsplit('@').next().unwrap_or("");
    let port = if scheme == "https" { 443 } else { 80 };
    let target = if hostport.contains(':') && !hostport.starts_with('[') && hostport.matches(':').count() == 1 {
        hostport.to_string()
    } else if hostport.starts_with('[') && hostport.contains("]:") {
        hostport.to_string()
    } else {
        format!("{hostport}:{port}")
    };
    let addrs = target.to_socket_addrs().map_err(|e| format!("cannot resolve {hostport}: {e}"))?;
    for a in addrs {
        if is_private(a.ip()) {
            return Err(format!("refused: {hostport} resolves to a private address"));
        }
    }
    Ok(())
}

fn strip_html(html: &str) -> String {
    let mut out = String::new();
    let lower = html.to_ascii_lowercase();
    let mut i = 0;
    let b = html.as_bytes();
    while i < b.len() {
        if b[i] == b'<' {
            let skip_tag = ["script", "style"].iter().find(|t| lower[i..].starts_with(&format!("<{t}")));
            if let Some(t) = skip_tag {
                let close = format!("</{t}");
                i = lower[i..].find(&close).map_or(b.len(), |p| i + p);
            }
            while i < b.len() && b[i] != b'>' {
                i += 1;
            }
            i += 1;
            out.push(' ');
        } else {
            let ch = html[i..].chars().next().unwrap();
            out.push(ch);
            i += ch.len_utf8();
        }
    }
    let out = out.replace("&amp;", "&").replace("&lt;", "<").replace("&gt;", ">").replace("&nbsp;", " ").replace("&quot;", "\"");
    // collapse whitespace; glue "Hello <b>world</b>" back together
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn fetch(url: &str, max: usize, allow_private: bool) -> Result<serde_json::Value, String> {
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .max_redirects(0)
        .http_status_as_error(false)
        .timeout_global(Some(std::time::Duration::from_secs(30)))
        .build()
        .into();
    let mut cur = url.to_string();
    for _ in 0..6 {
        check(&cur, allow_private)?;
        let resp = agent.get(&cur).call().map_err(|e| format!("fetch failed: {e}"))?;
        let status = resp.status().as_u16();
        if (300..400).contains(&status) {
            let loc = resp.headers().get("location").and_then(|v| v.to_str().ok()).ok_or("redirect without location")?;
            cur = if loc.contains("://") { loc.to_string() } else {
                let base = cur.split('?').next().unwrap_or(&cur);
                let root = base.splitn(4, '/').take(3).collect::<Vec<_>>().join("/");
                format!("{root}{}", if loc.starts_with('/') { loc.to_string() } else { format!("/{loc}") })
            };
            continue;
        }
        let ctype = resp.headers().get("content-type").and_then(|v| v.to_str().ok()).unwrap_or("").to_string();
        let mut buf = Vec::new();
        resp.into_body().into_reader().take(max as u64 + 1).read_to_end(&mut buf).map_err(|e| e.to_string())?;
        let truncated = buf.len() > max;
        buf.truncate(max);
        let raw = String::from_utf8_lossy(&buf).to_string();
        let text = if ctype.contains("html") { strip_html(&raw) } else { raw };
        return Ok(serde_json::json!({"ok": true, "status": status, "content_type": ctype, "text": text, "truncated": truncated, "url": cur}));
    }
    Err("too many redirects".into())
}

fn main() {
    serve("web.fetch", "tool", &mut |method, params| match method {
        "tool.call" => {
            let Some(url) = params["args"]["url"].as_str() else {
                return serde_json::json!({"$error": "url is required"});
            };
            let max = params["args"]["max_bytes"].as_u64().map_or(DEFAULT_MAX, |n| n as usize);
            let allow = std::env::var("HS_WEB_ALLOW_PRIVATE").is_ok_and(|v| v == "1");
            match fetch(url, max, allow) {
                Ok(v) => v,
                Err(e) => serde_json::json!({"$error": e}),
            }
        }
        _ => serde_json::json!({"$error": "unknown method"}),
    });
}
