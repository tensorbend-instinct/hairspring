//! web.search (dsh parity: web_search). Provider: Brave Search API
//! (keyless scraping is blocked from datacenter IPs: DuckDuckGo html returns an
//! anomaly challenge, Mojeek 403). Key from HS_BRAVE_API_KEY or
//! HS_BRAVE_API_KEY_FILE; endpoint override HS_SEARCH_URL for tests.
use std::io::{Read, Write};
use std::net::TcpListener;
use std::process::{Command, Stdio};

fn call(envs: &[(&str, &str)], args: serde_json::Value) -> serde_json::Value {
    let mut c = Command::new(env!("CARGO_BIN_EXE_hs-plugin-websearch"));
    c.env_remove("HS_BRAVE_API_KEY").env_remove("HS_BRAVE_API_KEY_FILE");
    for (k, v) in envs {
        c.env(k, v);
    }
    let mut p = c.stdin(Stdio::piped()).stdout(Stdio::piped()).spawn().unwrap();
    let req = serde_json::json!({"id":1,"method":"tool.call","params":{"args":args}});
    let mut si = p.stdin.take().unwrap();
    si.write_all(format!("{req}\n").as_bytes()).unwrap();
    drop(si);
    let out = p.wait_with_output().unwrap();
    let line = String::from_utf8_lossy(&out.stdout).lines().next().unwrap().to_string();
    let v: serde_json::Value = serde_json::from_str(&line).unwrap();
    if v.get("error").is_some() { serde_json::json!({"$error": v["error"]}) } else { v["result"].clone() }
}

fn serve(body: &'static str) -> (String, std::sync::mpsc::Receiver<String>) {
    let l = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = l.local_addr().unwrap().port();
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let (mut s, _) = l.accept().unwrap();
        let mut buf = [0u8; 4096];
        let n = s.read(&mut buf).unwrap();
        tx.send(String::from_utf8_lossy(&buf[..n]).to_string()).unwrap();
        let resp = format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
        s.write_all(resp.as_bytes()).unwrap();
    });
    (format!("http://127.0.0.1:{port}/search"), rx)
}

#[test]
fn s1_parses_results_and_sends_key_and_query() {
    let (url, rx) = serve(r#"{"web":{"results":[{"title":"Rust","url":"https://rust-lang.org","description":"A language"},{"title":"Docs","url":"https://doc.rust-lang.org","description":"Docs <strong>here</strong>"}]}}"#);
    let r = call(&[("HS_SEARCH_URL", &url), ("HS_BRAVE_API_KEY", "k-123")], serde_json::json!({"query":"rust lang","count":2}));
    assert_eq!(r["ok"], true, "{r}");
    assert_eq!(r["results"][0]["title"], "Rust");
    assert_eq!(r["results"][0]["url"], "https://rust-lang.org");
    assert_eq!(r["results"][1]["snippet"], "Docs here");
    let req = rx.recv().unwrap().to_lowercase();
    assert!(req.contains("q=rust%20lang") || req.contains("q=rust+lang"), "{req}");
    assert!(req.contains("x-subscription-token: k-123"), "{req}");
}

#[test]
fn s2_missing_key_says_what_is_needed() {
    let r = call(&[("HS_SEARCH_URL", "http://127.0.0.1:1/x")], serde_json::json!({"query":"x"}));
    assert!(r["$error"].as_str().unwrap().contains("HS_BRAVE_API_KEY"), "{r}");
}

#[test]
fn s3_requires_query_and_is_wired() {
    let r = call(&[("HS_BRAVE_API_KEY", "k")], serde_json::json!({}));
    assert!(r["$error"].as_str().unwrap().contains("query"), "{r}");
    let t = hs_loop::toolschema::schema_for("web.search", "apply").expect("schema");
    assert_eq!(t["function"]["parameters"]["required"][0], "query");
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    assert!(std::fs::read_to_string(root.join("install.sh")).unwrap().contains("hs-plugin-websearch"));
    assert!(std::fs::read_to_string(root.join("hairspring.example.toml")).unwrap().contains("name = \"web.search\""));
}
