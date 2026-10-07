//! A slow critic call is allowed to complete past the former per-call watchdog.
use hs_loop::critic::{CriticModel, ProviderCritic};
use std::io::{Read, Write};
use std::net::TcpListener;
use std::time::{Duration,Instant};
#[test]
fn slow_critic_response_survives_old_watchdog() {
 let listener=TcpListener::bind("127.0.0.1:0").unwrap();
 let url=format!("http://{}/chat/completions",listener.local_addr().unwrap());
 let worker=std::thread::spawn(move||{
  let (mut stream,_)=listener.accept().unwrap();
  stream.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
  let mut b=[0u8;8192];let _=stream.read(&mut b);
  std::thread::sleep(Duration::from_millis(1200));
  let body=r#"{"usage":{"prompt_tokens":1,"completion_tokens":1},"choices":[{"message":{"content":"ok"}}]}"#;
  let response=format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",body.len(),body);
  let _=stream.write_all(response.as_bytes());
 });
 let mut p=hs_loop::realmodel::deepseek();p.default_base_url=url;p.base_url_env="HS_CRITIC_NOKILL_UNUSED_URL".into();p.key_env="HS_CRITIC_NOKILL_KEY".into();p.key_file_env="HS_CRITIC_NOKILL_KEY_FILE".into();
 let dir=tempfile::tempdir().unwrap();let key=dir.path().join("key");std::fs::write(&key,"fake-key").unwrap();unsafe {std::env::set_var("HS_CRITIC_NOKILL_KEY_FILE",&key);std::env::set_var("HS_REALMODEL_CALL_TIMEOUT_SECS","1");}
 let mut c=ProviderCritic::for_provider(p).unwrap();let start=Instant::now();let result=c.step(&[serde_json::json!({"role":"user","content":"test"})]);
 assert!(start.elapsed()>=Duration::from_millis(1200),"critic cut off early: {result:?}");assert!(result.is_ok(),"{result:?}");worker.join().unwrap();
}
