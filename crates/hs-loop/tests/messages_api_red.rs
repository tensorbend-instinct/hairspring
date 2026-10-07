//! Messages transport: conversion, schema fidelity, usage, and hostile frames.
use serde_json::json;
#[test]
fn request_matches_messages_shape_and_preserves_tool_pairs(){
 let b=hs_loop::messagesapi::from_chat(&json!({"model":"deepseek-flash","max_tokens":393216,"messages":[{"role":"system","content":"sys"},{"role":"user","content":"task"},{"role":"assistant","tool_calls":[{"id":"c1","function":{"name":"read","arguments":"{\"file_path\":\"a\"}"}}]},{"role":"tool","tool_call_id":"c1","content":"1: hello"}],"tools":[{"type":"function","function":{"name":"read","description":"Read file","parameters":{"type":"object","properties":{"file_path":{"type":"string"}},"required":["file_path"]}}}]})).unwrap();
 assert_eq!(b["system"],"sys");assert_eq!(b["thinking"]["type"],"enabled");assert_eq!(b["output_config"]["effort"],"high");
 assert_eq!(b["tools"][0]["input_schema"]["required"],json!(["file_path"]));
 assert_eq!(b["messages"][1]["content"][0]["type"],"tool_use");assert_eq!(b["messages"][2]["content"][0]["tool_use_id"],"c1");
}
#[test]
fn malformed_or_orphan_tool_history_fails_closed(){
 for ms in [json!([{"role":"tool","tool_call_id":"missing","content":"ok"}]),json!([{"role":"assistant","tool_calls":[{"id":"a","function":{"name":"read","arguments":"broken"}}]}])]{assert!(hs_loop::messagesapi::from_chat(&json!({"messages":ms})).is_err());}
}
#[test]
fn messages_response_normalizes_tools_cache_and_served_identity(){
 let r=hs_loop::messagesapi::to_chat(&json!({"model":"served-flash","stop_reason":"tool_use","content":[{"type":"thinking","thinking":"check"},{"type":"tool_use","id":"a","name":"read","input":{"file_path":"a"}}],"usage":{"input_tokens":10,"cache_read_input_tokens":90,"output_tokens":7}})).unwrap();
 assert_eq!(r["model"],"served-flash");assert_eq!(r["usage"]["prompt_tokens"],100);assert_eq!(r["usage"]["prompt_cache_hit_tokens"],90);assert_eq!(r["choices"][0]["message"]["reasoning_content"],"check");
}
#[test]
fn incomplete_and_in_band_error_streams_are_not_success(){
 assert!(hs_loop::messagesapi::assemble_sse("event: message_start\ndata: {\"type\":\"message_start\",\"message\":{}}\n\n").is_err());
 assert!(hs_loop::messagesapi::assemble_sse("event: error\ndata: {\"type\":\"error\",\"error\":{\"message\":\"bad\"}}\n\n").is_err());
}
#[test]
fn full_stream_tool_fragments_and_cache_usage_are_assembled(){
 let s=concat!("event: message_start\ndata: {\"type\":\"message_start\",\"message\":{\"model\":\"served\",\"usage\":{\"input_tokens\":4,\"cache_read_input_tokens\":6}}}\n\n",
 "event: content_block_start\ndata: {\"type\":\"content_block_start\",\"index\":0,\"content_block\":{\"type\":\"tool_use\",\"id\":\"a\",\"name\":\"read\",\"input\":{}}}\n\n",
 "event: content_block_delta\ndata: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"input_json_delta\",\"partial_json\":\"{\\\"file_path\\\":\\\"a\\\"}\"}}\n\n",
 "event: message_delta\ndata: {\"type\":\"message_delta\",\"delta\":{\"stop_reason\":\"tool_use\"},\"usage\":{\"output_tokens\":2}}\n\n",
 "event: message_stop\ndata: {\"type\":\"message_stop\"}\n\n");
 let v=hs_loop::messagesapi::assemble_sse(s).unwrap();assert_eq!(v["model"],"served");assert_eq!(v["usage"]["prompt_tokens"],10);assert_eq!(v["choices"][0]["message"]["tool_calls"][0]["function"]["arguments"],"{\"file_path\":\"a\"}");
}
#[test]
fn messages_mock_checks_path_auth_body_and_real_model_identity(){
 use std::io::{BufRead,Read,Write};use std::net::TcpListener;
 let l=TcpListener::bind("127.0.0.1:0").unwrap();let url=format!("http://{}/anthropic/v1/messages",l.local_addr().unwrap());
 let h=std::thread::spawn(move||{let(s,_)=l.accept().unwrap();let mut r=std::io::BufReader::new(s);let mut first=String::new();r.read_line(&mut first).unwrap();let mut n=0;let mut hs=String::new();loop{let mut line=String::new();r.read_line(&mut line).unwrap();if line.trim().is_empty(){break;}if line.to_lowercase().starts_with("content-length:"){n=line.split(':').nth(1).unwrap().trim().parse().unwrap();}hs.push_str(&line);}let mut buf=vec![0;n];r.read_exact(&mut buf).unwrap();let b:serde_json::Value=serde_json::from_slice(&buf).unwrap();let out=r#"{"id":"a","type":"message","model":"actual-served-flash","stop_reason":"tool_use","content":[{"type":"tool_use","id":"a","name":"read","input":{"file_path":"a"}}],"usage":{"input_tokens":4,"cache_read_input_tokens":6,"output_tokens":2}}"#;write!(r.get_mut(),"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",out.len(),out).unwrap();(first,hs,b)});
 let mut p=hs_loop::realmodel::deepseek();p.default_base_url=url;p.base_url_env="UNSET_MSG_TEST_URL".into();p.key_env="UNSET_MSG_TEST_KEY".into();p.key_file_env="UNSET_MSG_TEST_KEY_FILE".into();let dir=tempfile::tempdir().unwrap();let key=dir.path().join("key");std::fs::write(&key,"fake").unwrap();p.key_file_env=String::new();
 // Dedicated test-only environment names, no real credential.
 unsafe{std::env::set_var("UNSET_MSG_TEST_KEY","fake");std::env::set_var("HS_REALMODEL_MAX_ATTEMPTS","1");}p.key_env="UNSET_MSG_TEST_KEY".into();
 let result=hs_loop::realmodel::call_messages(&p,&json!([{"role":"user","content":"read"}]),Some(&json!([{"type":"function","function":{"name":"read","description":"Read","parameters":{"type":"object"}}}])));
 unsafe{std::env::remove_var("UNSET_MSG_TEST_KEY");std::env::remove_var("HS_REALMODEL_MAX_ATTEMPTS");}
 let(first,headers,body)=h.join().unwrap();assert!(first.contains("/anthropic/v1/messages"));assert!(headers.to_lowercase().contains("x-api-key: fake"));assert!(headers.to_lowercase().contains("anthropic-version: 2023-06-01"));assert_eq!(body["tools"][0]["name"],"read");let v=result.unwrap();assert_eq!(v["served_model"],"actual-served-flash");assert_eq!(v["input_tokens"],10);
}
#[test]
fn streaming_mock_preserves_fragments_and_no_watchdog(){
 use std::io::{BufRead,Read,Write};use std::net::TcpListener;
 let l=TcpListener::bind("127.0.0.1:0").unwrap();let url=format!("http://{}/anthropic/v1/messages",l.local_addr().unwrap());
 let h=std::thread::spawn(move||{let(s,_)=l.accept().unwrap();let mut r=std::io::BufReader::new(s);let mut n=0;loop{let mut line=String::new();r.read_line(&mut line).unwrap();if line.trim().is_empty(){break;}if line.to_lowercase().starts_with("content-length:"){n=line.split(':').nth(1).unwrap().trim().parse().unwrap();}}let mut b=vec![0;n];r.read_exact(&mut b).unwrap();let b:serde_json::Value=serde_json::from_slice(&b).unwrap();assert_eq!(b["stream"],true);let out=concat!("event: message_start\ndata: {\"type\":\"message_start\",\"message\":{\"model\":\"served\",\"usage\":{\"input_tokens\":1}}}\n\n","event: content_block_start\ndata: {\"type\":\"content_block_start\",\"index\":0,\"content_block\":{\"type\":\"text\",\"text\":\"\"}}\n\n","event: content_block_delta\ndata: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\"ok\"}}\n\n","event: message_delta\ndata: {\"type\":\"message_delta\",\"delta\":{\"stop_reason\":\"end_turn\"},\"usage\":{\"output_tokens\":1}}\n\n","event: message_stop\ndata: {\"type\":\"message_stop\"}\n\n");write!(r.get_mut(),"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",out.len()).unwrap();for chunk in out.as_bytes().chunks(11){r.get_mut().write_all(chunk).unwrap();}});
 let mut p=hs_loop::realmodel::deepseek();p.default_base_url=url;p.base_url_env="UNSET_MSG_STREAM_URL".into();p.key_env="UNSET_MSG_STREAM_KEY".into();p.key_file_env="UNSET_MSG_STREAM_KEY_FILE".into();unsafe{std::env::set_var("UNSET_MSG_STREAM_KEY","fake");}let seen=std::sync::Mutex::new(String::new());let result=hs_loop::realmodel::call_streaming(&p,"test",None,&|d|seen.lock().unwrap().push_str(d));unsafe{std::env::remove_var("UNSET_MSG_STREAM_KEY");}h.join().unwrap();assert_eq!(result.unwrap()["served_model"],"served");assert_eq!(*seen.lock().unwrap(),"ok");
}
#[test]
fn deepseek_defaults_match_dsh_messages_route(){let p=hs_loop::realmodel::deepseek();assert_eq!(p.default_base_url,"https://api.deepseek.com/anthropic/v1/messages");assert_eq!(p.default_model,"deepseek-flash");}
#[test]
fn stream_rejects_delta_before_start_and_malformed_tool_json(){
 let orphan="event: content_block_delta\ndata: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\"fake\"}}\n\n";assert!(hs_loop::messagesapi::assemble_sse(orphan).is_err());
 let s=concat!("event: message_start\ndata: {\"type\":\"message_start\",\"message\":{\"usage\":{\"input_tokens\":1}}}\n\n","event: content_block_start\ndata: {\"type\":\"content_block_start\",\"index\":0,\"content_block\":{\"type\":\"tool_use\",\"id\":\"a\",\"name\":\"read\",\"input\":{}}}\n\n","event: content_block_delta\ndata: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"input_json_delta\",\"partial_json\":\"not-json\"}}\n\n","event: message_stop\ndata: {\"type\":\"message_stop\"}\n\n");assert!(hs_loop::messagesapi::assemble_sse(s).is_err());
}
#[test]
fn native_assistant_thinking_signature_and_tools_are_preserved(){
 let blocks=json!([{"type":"thinking","thinking":"check","signature":"sig"},{"type":"tool_use","id":"a","name":"term_exec","input":{"command":"true"}}]);
 let b=hs_loop::messagesapi::from_chat(&json!({"messages":[{"role":"assistant","content":blocks,"tool_calls":[{"id":"a","function":{"name":"term_exec","arguments":"{\"command\":\"true\"}"}}]},{"role":"tool","tool_call_id":"a","content":"ok"}]})).unwrap();assert_eq!(b["messages"][0]["content"],blocks);
}
#[test]
fn parallel_tool_results_share_one_immediately_following_user_message(){
 let b=hs_loop::messagesapi::from_chat(&json!({"messages":[{"role":"assistant","tool_calls":[{"id":"a","function":{"name":"bash","arguments":"{}"}},{"id":"b","function":{"name":"bash","arguments":"{}"}}]},{"role":"tool","tool_call_id":"a","content":"A"},{"role":"tool","tool_call_id":"b","content":"B"}]})).unwrap();
 assert_eq!(b["messages"].as_array().unwrap().len(),2);
 assert_eq!(b["messages"][1]["content"].as_array().unwrap().len(),2);
}
#[test]
fn author_rejects_multiple_native_calls_instead_of_silently_discarding_them(){
 let v=json!({"model":"served","choices":[{"message":{"tool_calls":[{"id":"a","function":{"name":"repo_read","arguments":"{}"}},{"id":"b","function":{"name":"repo_read","arguments":"{}"}}]}}]});
 assert!(hs_loop::realmodel::parse_response(&hs_loop::realmodel::deepseek(),&v).is_err());
}
#[test]
fn messages_stream_exposes_protocol_completion_without_http_eof(){let mut p=hs_loop::messagesapi::Stream::default();p.push(b"event: message_start\ndata: {\"type\":\"message_start\",\"message\":{\"usage\":{\"input_tokens\":1}}}\n\nevent: message_stop\ndata: {\"type\":\"message_stop\"}\n\n",&mut |_|{}).unwrap();assert!(p.is_done());}
#[test]
fn streaming_returns_at_message_stop_while_server_keeps_connection_open(){
 use std::io::{BufRead,Read,Write};use std::net::TcpListener;
 let l=TcpListener::bind("127.0.0.1:0").unwrap();let url=format!("http://{}/anthropic/v1/messages",l.local_addr().unwrap());let (release,held)=std::sync::mpsc::channel();
 let server=std::thread::spawn(move||{let(s,_)=l.accept().unwrap();let mut r=std::io::BufReader::new(s);let mut n=0;loop{let mut line=String::new();r.read_line(&mut line).unwrap();if line.trim().is_empty(){break}if line.to_lowercase().starts_with("content-length:"){n=line.split(':').nth(1).unwrap().trim().parse().unwrap()}}let mut b=vec![0;n];r.read_exact(&mut b).unwrap();let data=concat!("event: message_start\ndata: {\"type\":\"message_start\",\"message\":{\"model\":\"served\",\"usage\":{\"input_tokens\":1}}}\n\n","event: content_block_start\ndata: {\"type\":\"content_block_start\",\"index\":0,\"content_block\":{\"type\":\"text\",\"text\":\"OK\"}}\n\n","event: message_stop\ndata: {\"type\":\"message_stop\"}\n\n");write!(r.get_mut(),"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nTransfer-Encoding: chunked\r\nConnection: keep-alive\r\n\r\n{:x}\r\n{}\r\n",data.len(),data).unwrap();r.get_mut().flush().unwrap();held.recv().unwrap();write!(r.get_mut(),"0\r\n\r\n").ok();});
 let mut p=hs_loop::realmodel::deepseek();p.default_base_url=url;p.base_url_env="UNSET_KEEP_OPEN_URL".into();p.key_env="KEEP_OPEN_TEST_KEY".into();p.key_file_env="UNSET_KEEP_OPEN_KEY_FILE".into();unsafe{std::env::set_var("KEEP_OPEN_TEST_KEY","fake");}let(tx,rx)=std::sync::mpsc::channel();let worker=std::thread::spawn(move||{tx.send(hs_loop::realmodel::call_streaming(&p,"OK",None,&|_|{})).unwrap();});let result=rx.recv_timeout(std::time::Duration::from_secs(2));release.send(()).unwrap();server.join().unwrap();worker.join().unwrap();unsafe{std::env::remove_var("KEEP_OPEN_TEST_KEY");}assert!(result.is_ok(),"waited for HTTP EOF after message_stop");assert_eq!(result.unwrap().unwrap()["completion"],"OK");
}
