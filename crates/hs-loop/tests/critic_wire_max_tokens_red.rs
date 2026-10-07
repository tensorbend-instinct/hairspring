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
    assert_eq!(body["model"], "deepseek-flash");
    assert_eq!(body["max_tokens"], 393216, "critic must request the same output cap as author and OpenHands");
}
#[test]
fn messages_critic_preserves_native_parallel_history_and_served_identity(){
 use std::io::{BufRead,BufReader}; use serde_json::{json,Value};
 let l=TcpListener::bind("127.0.0.1:0").unwrap();let url=format!("http://{}/anthropic/v1/messages",l.local_addr().unwrap());
 let blocks=json!([{"type":"thinking","thinking":"probe","signature":"provider-signature"},{"type":"tool_use","id":"a","name":"term_exec","input":{"command":"true"}},{"type":"tool_use","id":"b","name":"term_exec","input":{"command":"pwd"}}]);let expected=blocks.clone();
 let h=std::thread::spawn(move||{let mut bodies=vec![];for i in 0..2 {let(s,_)=l.accept().unwrap();let mut r=BufReader::new(s);let mut n=0;loop{let mut line=String::new();r.read_line(&mut line).unwrap();if line.trim().is_empty(){break;}if line.to_lowercase().starts_with("content-length:"){n=line.split(':').nth(1).unwrap().trim().parse().unwrap();}}let mut bytes=vec![0;n];r.read_exact(&mut bytes).unwrap();bodies.push(serde_json::from_slice::<Value>(&bytes).unwrap());let content=if i==0 {blocks.clone()}else{json!([{"type":"text","text":"{\"refuted\":false,\"reason\":\"checked\"}"}])};let reply=json!({"model":"actual-served","content":content,"usage":{"input_tokens":1,"output_tokens":1}}).to_string();write!(r.get_mut(),"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",reply.len(),reply).unwrap();}bodies});
 let mut p=hs_loop::realmodel::deepseek();p.default_base_url=url;p.base_url_env="UNSET_CRITIC_PARALLEL_URL".into();p.key_env="UNSET_CRITIC_PARALLEL_KEY".into();p.key_file_env="CRITIC_PARALLEL_KEY_FILE".into();let d=tempfile::tempdir().unwrap();let k=d.path().join("key");std::fs::write(&k,"fake").unwrap();unsafe{std::env::set_var("CRITIC_PARALLEL_KEY_FILE",&k);}
 let mut critic=ProviderCritic::for_provider(p).unwrap();let mut ms=vec![json!({"role":"user","content":"probe"})];critic.step(&ms).unwrap();assert_eq!(critic.served_model(),"actual-served");ms.push(json!({"role":"assistant","content":critic.assistant_content().unwrap()}));ms.push(json!({"role":"tool","tool_call_id":"a","content":"ok"}));ms.push(json!({"role":"tool","tool_call_id":"b","content":"cwd"}));critic.step(&ms).unwrap();unsafe{std::env::remove_var("CRITIC_PARALLEL_KEY_FILE");}
 let bodies=h.join().unwrap();assert_eq!(bodies[1]["messages"][1]["content"],expected);assert_eq!(bodies[1]["messages"][2]["content"].as_array().unwrap().len(),2);
}
