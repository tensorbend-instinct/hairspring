//! Hermetic tests for the real-model adapters: a mock chat-completions
//! server checks the auth header and request shape; adapters parse usage
//! and compute cost. No real keys, no network. Also covers key-file
//! loading and JSON extraction from fenced/prose-wrapped output.

use hs_loop::realmodel::*;

/// Env vars are process-global: tests that point providers at mock servers
/// must not run concurrently.
static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;
use std::sync::{Mutex, OnceLock};

fn env_lock() -> &'static Mutex<()> {
    static L: OnceLock<Mutex<()>> = OnceLock::new();
    L.get_or_init(|| Mutex::new(()))
}

struct Mock {
    url: String,
    got_auth: std::sync::mpsc::Receiver<String>,
    got_body: std::sync::mpsc::Receiver<String>,
}

fn mock_chat_server(response_body: &'static str) -> Mock {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let (atx, arx) = std::sync::mpsc::channel();
    let (btx, brx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let (stream, _) = listener.accept().unwrap();
        let mut reader = BufReader::new(stream);
        let mut content_len = 0usize;
        let mut auth = String::new();
        loop {
            let mut line = String::new();
            reader.read_line(&mut line).unwrap();
            let t = line.trim();
            if t.is_empty() {
                break;
            }
            if let Some(v) = t.to_ascii_lowercase().strip_prefix("content-length:") {
                content_len = v.trim().parse().unwrap();
            }
            if t.to_ascii_lowercase().starts_with("authorization:") {
                auth = t["authorization:".len()..].trim_start().to_string();
            }
        }
        let mut body = vec![0u8; content_len];
        reader.read_exact(&mut body).unwrap();
        atx.send(auth).unwrap();
        btx.send(String::from_utf8(body).unwrap()).unwrap();
        let resp = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            response_body.len(),
            response_body
        );
        reader.get_mut().write_all(resp.as_bytes()).unwrap();
    });
    Mock {
        url: format!("http://127.0.0.1:{port}/chat/completions"),
        got_auth: arx,
        got_body: brx,
    }
}

#[test]
fn extract_json_from_wrapped_output() {
    assert_eq!(extract_json_object("{\"a\":1}"), Some("{\"a\":1}"));
    let fenced = "Here you go:\n```json\n{\"tool\":\"answer.write\",\"args\":{\"path\":\"/x\",\"content\":\"a}b\"}}\n```\nDone.";
    let got = extract_json_object(fenced).unwrap();
    let v: serde_json::Value = serde_json::from_str(got).unwrap();
    assert_eq!(v["args"]["content"], "a}b");
    assert!(extract_json_object("no json here").is_none());
}

#[test]
fn adapters_against_mock_server() {
    let _g = ENV_LOCK.lock().unwrap();
    let _g = env_lock().lock().unwrap();

    // GLM: key from env, fenced completion, openai-style usage
    let glm_mock = mock_chat_server(
        r#"{"choices":[{"message":{"content":"```json\n{\"tool\":\"answer.write\",\"args\":{\"path\":\"/p\",\"content\":\"X\"}}\n```"}}],"usage":{"prompt_tokens":1000,"completion_tokens":250}}"#,
    );
    std::env::set_var(glm().base_url_env, &glm_mock.url);
    std::env::set_var(glm().key_env, "mock-glm-key");
    let out = call(&glm(), "MISSION: task-0\nANSWER_PATH: /p", None).unwrap();
    assert_eq!(glm_mock.got_auth.recv().unwrap(), "Bearer mock-glm-key");
    let body: serde_json::Value = serde_json::from_str(&glm_mock.got_body.recv().unwrap()).unwrap();
    assert_eq!(body["model"], "glm-5.3");
    assert_eq!(body["temperature"], 0);
    assert_eq!(
        out["completion"],
        "{\"tool\":\"answer.write\",\"args\":{\"path\":\"/p\",\"content\":\"X\"}}"
    );
    assert_eq!(out["input_tokens"], 1000);
    assert_eq!(out["output_tokens"], 250);
    // 1000 * 1.40 + 250 * 4.40 = 2500 micros
    assert_eq!(out["cost_usd_micros"], 2500);

    // DeepSeek: key from file, cache-hit-aware cost
    let dir = tempfile::tempdir().unwrap();
    let keyfile = dir.path().join("ds.key");
    std::fs::write(&keyfile, "mock-ds-key\n").unwrap();
    let ds_mock = mock_chat_server(
        r#"{"choices":[{"message":{"content":"{\"tool\":\"answer.write\",\"args\":{\"path\":\"/p\",\"content\":\"Y\"}}"}}],"usage":{"prompt_tokens":1000,"completion_tokens":250,"prompt_cache_hit_tokens":600,"prompt_cache_miss_tokens":400}}"#,
    );
    std::env::remove_var(deepseek().key_env);
    std::env::set_var(deepseek().key_file_env, &keyfile);
    std::env::set_var(deepseek().base_url_env, &ds_mock.url);
    let out = call(&deepseek(), "MISSION: task-1\nANSWER_PATH: /p", None).unwrap();
    assert_eq!(ds_mock.got_auth.recv().unwrap(), "Bearer mock-ds-key");
    let body: serde_json::Value = serde_json::from_str(&ds_mock.got_body.recv().unwrap()).unwrap();
    assert_eq!(body["model"], "deepseek-v4-flash");
    assert_eq!(out["cached_tokens"], 600);
    // 600*0.014 + 400*0.44 + 250*1.32 = 8.4 + 176 + 330 = 514.4 -> 514
    assert_eq!(out["cost_usd_micros"], 514);

    // missing key is an error that never contains a secret
    std::env::remove_var(glm().key_env);
    std::env::remove_var(glm().key_file_env);
    let e = call(&glm(), "x", None).unwrap_err();
    assert!(e.contains("no API key"), "{e}");
    assert!(!e.contains("mock-glm-key"));

    // provider 4xx surfaces status only, never the key
    std::env::set_var(deepseek().key_env, "mock-ds-key");
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    std::thread::spawn(move || {
        // serve every connection the client opens (retry-safe)
        while let Ok((mut s, _)) = listener.accept() {
            let mut buf = [0u8; 8192];
            let _ = s.read(&mut buf);
            let _ = s.write_all(
                b"HTTP/1.1 401 Unauthorized\r\nContent-Length: 2\r\nConnection: close\r\n\r\n{}",
            );
        }
    });
    std::env::set_var(
        deepseek().base_url_env,
        format!("http://127.0.0.1:{port}/chat/completions"),
    );
    let e = call(&deepseek(), "x", None).unwrap_err();
    assert!(e.contains("401"), "{e}");
    assert!(!e.contains("mock-ds-key"));
}

/// A provider that accepts the connection and never responds must be cut
/// off by the watchdog with the sentinel completion - the pre-fix behavior
/// hung the whole harness for 30+ minutes (observed live 2026-09-03).
#[test]
fn watchdog_cutoff_returns_sentinel_not_hang() {
    let _g = ENV_LOCK.lock().unwrap();
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    std::thread::spawn(move || {
        for s in listener.incoming().flatten() {
            // hold the connection open, say nothing, for far longer
            // than the watchdog
            std::thread::sleep(std::time::Duration::from_secs(30));
            drop(s);
        }
    });
    std::env::set_var(
        "HS_GLM_BASE_URL",
        format!("http://127.0.0.1:{port}/chat/completions"),
    );
    std::env::set_var("HS_GLM_API_KEY", "test-dummy-not-a-real-key");
    std::env::set_var("HS_REALMODEL_CALL_TIMEOUT_SECS", "2");
    let t0 = std::time::Instant::now();
    let r = hs_loop::realmodel::call(&hs_loop::realmodel::glm(), "hi", None).unwrap();
    assert!(
        t0.elapsed() < std::time::Duration::from_secs(15),
        "watchdog did not cut the hung call: {:?}",
        t0.elapsed()
    );
    assert_eq!(
        r["completion"],
        hs_loop::realmodel::WATCHDOG_SENTINEL,
        "hung provider must yield the sentinel (feedback), not an error"
    );
    assert_eq!(r["cost_usd_micros"], 0);
}
