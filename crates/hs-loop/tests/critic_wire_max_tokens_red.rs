//! Capture the actual provider critic HTTP request, not just its source shape.
use hs_loop::critic::{CriticModel, ProviderCritic};
use std::io::{Read, Write};
use std::net::TcpListener;
use std::time::Duration;

#[test]
fn critic_requests_full_flash_output_budget_on_wire() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}/chat/completions", listener.local_addr().unwrap());
    let capture = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        stream.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
        let mut request = Vec::new();
        let mut buf = [0u8; 8192];
        let head_end = loop {
            let n = stream.read(&mut buf).unwrap();
            assert!(n > 0, "headers ended early");
            request.extend_from_slice(&buf[..n]);
            if let Some(i) = request.windows(4).position(|v| v == b"\r\n\r\n") { break i + 4; }
        };
        let headers = String::from_utf8_lossy(&request[..head_end]);
        let length: usize = headers.lines().find_map(|line| line.to_ascii_lowercase()
            .strip_prefix("content-length:").and_then(|x| x.trim().parse().ok()))
            .expect("content-length");
        while request.len() - head_end < length {
            let n = stream.read(&mut buf).unwrap();
            assert!(n > 0, "body ended early");
            request.extend_from_slice(&buf[..n]);
        }
        let body: serde_json::Value = serde_json::from_slice(&request[head_end..head_end+length]).unwrap();
        let response = r#"{"usage":{"prompt_tokens":1,"completion_tokens":1},"choices":[{"message":{"content":"ok"}}]}"#;
        write!(stream, "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}", response.len(), response).unwrap();
        body
    });
    let mut p = hs_loop::realmodel::deepseek();
    p.default_base_url = url;
    p.base_url_env = "HS_CRITIC_WIRE_TEST_UNUSED_URL".into();
    p.key_env = "HS_CRITIC_WIRE_TEST_KEY".into();
    p.key_file_env = "HS_CRITIC_WIRE_TEST_KEY_FILE".into();
    // Do not alter process-global environment; provide a temporary file via
    // the provider's normal load_key config path instead.
    let home = tempfile::tempdir().unwrap();
    let key_file = home.path().join("key");
    std::fs::write(&key_file, "fake-wire-key").unwrap();
    unsafe { std::env::set_var("HS_CRITIC_WIRE_TEST_KEY_FILE", &key_file); }
    let mut critic = ProviderCritic::for_provider(p).unwrap();
    critic.step(&[serde_json::json!({"role":"user","content":"probe"})]).unwrap();
    let body = capture.join().unwrap();
    assert_eq!(body["model"], "deepseek-v4-pro");
    assert_eq!(body["max_tokens"], 393216, "critic must request the same output cap as author and OpenHands");
}
