//! Hermetic tests for the real-model adapters: a mock chat-completions
//! server checks the auth header and request shape; adapters parse usage
//! and compute cost. No real keys, no network. Also covers key-file
//! loading and JSON extraction from fenced/prose-wrapped output.

use hs_loop::realmodel::*;
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
    let _g = env_lock().lock().unwrap();

    // GLM: key from env, fenced completion, openai-style usage
    let glm_mock = mock_chat_server(
        r#"{"choices":[{"message":{"content":"```json\n{\"tool\":\"answer.write\",\"args\":{\"path\":\"/p\",\"content\":\"X\"}}\n```"}}],"usage":{"prompt_tokens":1000,"completion_tokens":250}}"#,
    );
    std::env::set_var(GLM.base_url_env, &glm_mock.url);
    std::env::set_var(GLM.key_env, "mock-glm-key");
    let out = call(&GLM, "MISSION: task-0\nANSWER_PATH: /p").unwrap();
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
    std::env::remove_var(DEEPSEEK.key_env);
    std::env::set_var(DEEPSEEK.key_file_env, &keyfile);
    std::env::set_var(DEEPSEEK.base_url_env, &ds_mock.url);
    let out = call(&DEEPSEEK, "MISSION: task-1\nANSWER_PATH: /p").unwrap();
    assert_eq!(ds_mock.got_auth.recv().unwrap(), "Bearer mock-ds-key");
    let body: serde_json::Value = serde_json::from_str(&ds_mock.got_body.recv().unwrap()).unwrap();
    assert_eq!(body["model"], "deepseek-v4-flash");
    assert_eq!(out["cached_tokens"], 600);
    // 600*0.014 + 400*0.44 + 250*1.32 = 8.4 + 176 + 330 = 514.4 -> 514
    assert_eq!(out["cost_usd_micros"], 514);

    // missing key is an error that never contains a secret
    std::env::remove_var(GLM.key_env);
    std::env::remove_var(GLM.key_file_env);
    let e = call(&GLM, "x").unwrap_err();
    assert!(e.contains("no API key"), "{e}");
    assert!(!e.contains("mock-glm-key"));

    // provider 4xx surfaces status only, never the key
    std::env::set_var(DEEPSEEK.key_env, "mock-ds-key");
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    std::thread::spawn(move || {
        let (mut s, _) = listener.accept().unwrap();
        let mut buf = [0u8; 4096];
        let _ = s.read(&mut buf);
        s.write_all(
            b"HTTP/1.1 401 Unauthorized\r\nContent-Length: 2\r\nConnection: close\r\n\r\n{}",
        )
        .unwrap();
    });
    std::env::set_var(
        DEEPSEEK.base_url_env,
        format!("http://127.0.0.1:{port}/chat/completions"),
    );
    let e = call(&DEEPSEEK, "x").unwrap_err();
    assert!(e.contains("401"), "{e}");
    assert!(!e.contains("mock-ds-key"));
}
