//! RED: 429 resilience (user order 2026-09-05): honor the provider's
//! Retry-After header, exponential backoff otherwise, generous attempt
//! budget (HS_REALMODEL_MAX_ATTEMPTS, default 12) - a 429 should nearly
//! never kill a mission. Fatal 4xx (contract bugs like 400) fail fast.

use hs_loop::realmodel::*;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

fn env_lock() -> &'static Mutex<()> {
    static L: OnceLock<Mutex<()>> = OnceLock::new();
    L.get_or_init(|| Mutex::new(()))
}

/// Mock server answering each accepted connection with the next scripted
/// response; every accepted connection bumps the shared hit counter.
fn scripted_server(responses: Vec<String>, hits: Arc<AtomicUsize>) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    std::thread::spawn(move || {
        for resp in responses {
            if let Ok((stream, _)) = listener.accept() {
                hits.fetch_add(1, Ordering::SeqCst);
                let mut reader = BufReader::new(stream);
                let mut content_len = 0usize;
                loop {
                    let mut line = String::new();
                    if reader.read_line(&mut line).unwrap_or(0) == 0 {
                        break;
                    }
                    let t = line.trim();
                    if t.is_empty() {
                        break;
                    }
                    if let Some(v) = t.to_ascii_lowercase().strip_prefix("content-length:") {
                        content_len = v.trim().parse().unwrap_or(0);
                    }
                }
                if content_len > 0 {
                    let mut b = vec![0u8; content_len];
                    let _ = reader.read_exact(&mut b);
                }
                let _ = reader.get_mut().write_all(resp.as_bytes());
            }
        }
    });
    format!("http://127.0.0.1:{port}/chat/completions")
}

const R429_RA1: &str =
    "HTTP/1.1 429 Too Many Requests\r\nRetry-After: 1\r\nContent-Length: 0\r\nConnection: close\r\n\r\n";
const R429: &str =
    "HTTP/1.1 429 Too Many Requests\r\nContent-Length: 0\r\nConnection: close\r\n\r\n";
const R400: &str = "HTTP/1.1 400 Bad Request\r\nContent-Length: 0\r\nConnection: close\r\n\r\n";
fn r200() -> String {
    let body = r#"{"choices":[{"message":{"content":"ok"}}],"usage":{"prompt_tokens":1,"completion_tokens":1}}"#;
    format!(
        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        body.len(),
        body
    )
}

fn test_provider(url: &str) -> Provider {
    Provider {
        name: "rt429".into(),
        base_url_env: "HS_RT429_BASE_URL".into(),
        default_base_url: url.into(),
        model_env: "HS_RT429_MODEL".into(),
        default_model: "test-model".into(),
        key_env: "HS_RT429_API_KEY".into(),
        key_file_env: "HS_RT429_API_KEY_FILE".into(),
        price_in_env: "HS_RT429_PRICE_IN_MICROS".into(),
        default_in_micros: 0.0,
        price_cached_env: "HS_RT429_PRICE_CACHED_MICROS".into(),
        default_cached_micros: 0.0,
        price_out_env: "HS_RT429_PRICE_OUT_MICROS".into(),
        default_out_micros: 0.0,
        extra_body_json_env: "HS_RT429_EXTRA_BODY_JSON".into(),
        default_extra_body_json: None,
        tool_choice_required: true,
    }
}

#[test]
fn retry_after_header_is_honored() {
    let _g = env_lock().lock().unwrap();
    std::env::remove_var("HS_REALMODEL_MAX_ATTEMPTS");
    std::env::remove_var("HS_REALMODEL_BACKOFF_BASE_SECS");
    std::env::set_var("HS_RT429_API_KEY", "k");
    let hits = Arc::new(AtomicUsize::new(0));
    let url = scripted_server(vec![R429_RA1.into(), R429_RA1.into(), r200()], hits.clone());
    let p = test_provider(&url);
    let t0 = std::time::Instant::now();
    let out = call(&p, "MISSION: t", None).expect("429s with Retry-After must be survived");
    let el = t0.elapsed().as_secs();
    assert_eq!(out["completion"], "ok");
    assert_eq!(hits.load(Ordering::SeqCst), 3, "two 429s then success");
    assert!(el >= 2, "two Retry-After:1 sleeps must elapse, took {el}s");
}

#[test]
fn fatal_4xx_fails_immediately() {
    let _g = env_lock().lock().unwrap();
    std::env::remove_var("HS_REALMODEL_MAX_ATTEMPTS");
    std::env::remove_var("HS_REALMODEL_BACKOFF_BASE_SECS");
    std::env::set_var("HS_RT429_API_KEY", "k");
    let hits = Arc::new(AtomicUsize::new(0));
    let url = scripted_server(vec![R400.into(), r200()], hits.clone());
    let p = test_provider(&url);
    let err = call(&p, "MISSION: t", None).expect_err("400 is a contract bug, not retryable");
    assert!(err.contains("400"), "error names the status: {err}");
    assert_eq!(hits.load(Ordering::SeqCst), 1, "no retries on a fatal 4xx");
}

#[test]
fn max_attempts_env_bounds_retries() {
    let _g = env_lock().lock().unwrap();
    std::env::set_var("HS_REALMODEL_MAX_ATTEMPTS", "2");
    std::env::set_var("HS_REALMODEL_BACKOFF_BASE_SECS", "0");
    std::env::set_var("HS_RT429_API_KEY", "k");
    let hits = Arc::new(AtomicUsize::new(0));
    let url = scripted_server(
        vec![R429.into(), R429.into(), R429.into(), r200()],
        hits.clone(),
    );
    let p = test_provider(&url);
    let err = call(&p, "MISSION: t", None).expect_err("budget exhausted");
    assert!(err.contains("429"), "final error names the status: {err}");
    assert_eq!(hits.load(Ordering::SeqCst), 2, "attempt budget respected");
    std::env::remove_var("HS_REALMODEL_MAX_ATTEMPTS");
    std::env::remove_var("HS_REALMODEL_BACKOFF_BASE_SECS");
}

#[test]
fn exponential_backoff_without_retry_after() {
    let _g = env_lock().lock().unwrap();
    std::env::remove_var("HS_REALMODEL_MAX_ATTEMPTS");
    std::env::set_var("HS_REALMODEL_BACKOFF_BASE_SECS", "1");
    std::env::set_var("HS_RT429_API_KEY", "k");
    let hits = Arc::new(AtomicUsize::new(0));
    let url = scripted_server(vec![R429.into(), R429.into(), r200()], hits.clone());
    let p = test_provider(&url);
    let t0 = std::time::Instant::now();
    let out = call(&p, "MISSION: t", None).expect("bare 429s backed off and survived");
    let el = t0.elapsed().as_secs();
    assert_eq!(out["completion"], "ok");
    assert_eq!(hits.load(Ordering::SeqCst), 3);
    assert!(el >= 3, "exponential 1s+2s sleeps must elapse, took {el}s");
    std::env::remove_var("HS_REALMODEL_BACKOFF_BASE_SECS");
}

#[test]
fn generous_default_attempt_budget() {
    assert!(
        max_attempts() >= 8,
        "a 429 should nearly never kill a mission"
    );
}
