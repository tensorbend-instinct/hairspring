//! RED contract for the web.fetch tool (dsh parity: web_fetch).
//! args {url, max_bytes?}. Only http/https. Private/loopback targets are
//! refused (SSRF guard) unless HS_WEB_ALLOW_PRIVATE=1 (tests only). Replies
//! {ok, status, content_type, text, truncated}. HTML is reduced to text.
use std::io::{Read, Write};
use std::net::TcpListener;
use std::process::{Command, Stdio};

fn call(envs: &[(&str, &str)], args: serde_json::Value) -> serde_json::Value {
    let bin = env!("CARGO_BIN_EXE_hs-plugin-webfetch");
    let mut c = Command::new(bin);
    for (k, v) in envs {
        c.env(k, v);
    }
    let mut p = c.stdin(Stdio::piped()).stdout(Stdio::piped()).spawn().unwrap();
    let req = serde_json::json!({"id":1,"method":"tool.call","params":{"args":args}});
    let mut stdin = p.stdin.take().unwrap();
    stdin.write_all(format!("{req}\n").as_bytes()).unwrap();
    drop(stdin);
    let out = p.wait_with_output().unwrap();
    let line = String::from_utf8_lossy(&out.stdout).lines().next().unwrap().to_string();
    let v: serde_json::Value = serde_json::from_str(&line).unwrap();
    if v.get("error").is_some() { serde_json::json!({"$error": v["error"]}) } else { v["result"].clone() }
}

fn serve_once(body: &'static str, ctype: &'static str) -> String {
    let l = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = l.local_addr().unwrap().port();
    std::thread::spawn(move || {
        let (mut s, _) = l.accept().unwrap();
        let mut buf = [0u8; 2048];
        let _ = s.read(&mut buf);
        let resp = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: {ctype}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        );
        s.write_all(resp.as_bytes()).unwrap();
    });
    format!("http://127.0.0.1:{port}/")
}

#[test]
fn w1_fetches_text_and_strips_html() {
    let url = serve_once("<html><head><style>x{}</style></head><body><h1>Title</h1><p>Hello <b>world</b></p><script>evil()</script></body></html>", "text/html");
    let r = call(&[("HS_WEB_ALLOW_PRIVATE", "1")], serde_json::json!({"url": url}));
    assert_eq!(r["ok"], true, "{r}");
    assert_eq!(r["status"], 200);
    let t = r["text"].as_str().unwrap();
    assert!(t.contains("Title") && t.contains("Hello world"), "{t}");
    assert!(!t.contains("evil()") && !t.contains("<h1>") && !t.contains("x{}"), "{t}");
}

#[test]
fn w2_truncates_at_max_bytes() {
    let url = serve_once("abcdefghijklmnopqrstuvwxyz", "text/plain");
    let r = call(&[("HS_WEB_ALLOW_PRIVATE", "1")], serde_json::json!({"url": url, "max_bytes": 10}));
    assert_eq!(r["truncated"], true, "{r}");
    assert_eq!(r["text"].as_str().unwrap().len(), 10);
}

#[test]
fn w3_refuses_loopback_by_default() {
    let url = serve_once("secret", "text/plain");
    let r = call(&[], serde_json::json!({"url": url}));
    assert!(r["$error"].as_str().unwrap().contains("private"), "{r}");
}

#[test]
fn w4_refuses_non_http_schemes() {
    for u in ["file:///etc/passwd", "ftp://example.com/x", "gopher://x"] {
        let r = call(&[("HS_WEB_ALLOW_PRIVATE", "1")], serde_json::json!({"url": u}));
        assert!(r["$error"].as_str().unwrap().contains("scheme"), "{u}: {r}");
    }
}

#[test]
fn w5_model_sees_an_authored_schema() {
    let t = hs_loop::toolschema::schema_for("web.fetch", "apply").expect("web.fetch schema");
    assert!(t["function"]["parameters"]["properties"]["url"].is_object(), "{t}");
    assert_eq!(t["function"]["parameters"]["required"][0], "url");
}

#[test]
fn w6_shipped_with_the_install_and_example_config() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let install = std::fs::read_to_string(root.join("install.sh")).unwrap();
    assert!(install.contains("hs-plugin-webfetch"), "install.sh must ship the plugin");
    let toml = std::fs::read_to_string(root.join("hairspring.example.toml")).unwrap();
    assert!(toml.contains("name = \"web.fetch\"") && toml.contains("hs-plugin-webfetch"));
}
