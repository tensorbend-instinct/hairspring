//! ask_user_question (dsh parity): the model asks, the surface answers.
//! Protocol: the plugin writes <HS_ASK_DIR>/q-<n>.json {question, options?}
//! and blocks until <HS_ASK_DIR>/a-<n>.json {answer} appears or the timeout
//! (HS_ASK_TIMEOUT_SECS) passes. Timeout is a clean error, never a hang.
use std::io::Write;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

fn spawn(dir: &std::path::Path, timeout: &str, args: serde_json::Value) -> std::process::Child {
    let mut p = Command::new(env!("CARGO_BIN_EXE_hs-plugin-askuser"))
        .env("HS_ASK_DIR", dir)
        .env("HS_ASK_TIMEOUT_SECS", timeout)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let req = serde_json::json!({"id":1,"method":"tool.call","params":{"args":args}});
    let mut si = p.stdin.take().unwrap();
    si.write_all(format!("{req}\n").as_bytes()).unwrap();
    drop(si);
    p
}

fn result(p: std::process::Child) -> serde_json::Value {
    let out = p.wait_with_output().unwrap();
    let line = String::from_utf8_lossy(&out.stdout).lines().next().unwrap().to_string();
    serde_json::from_str(&line).unwrap()
}

#[test]
fn a1_question_is_published_and_the_answer_comes_back() {
    let d = tempfile::tempdir().unwrap();
    let p = spawn(d.path(), "10", serde_json::json!({"question":"Which db?","options":["pg","sqlite"]}));
    let q = d.path().join("q-1.json");
    let t = Instant::now();
    while !q.exists() {
        assert!(t.elapsed() < Duration::from_secs(5), "question never published");
        std::thread::sleep(Duration::from_millis(20));
    }
    let qv: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(&q).unwrap()).unwrap();
    assert_eq!(qv["question"], "Which db?");
    assert_eq!(qv["options"][1], "sqlite");
    std::fs::write(d.path().join("a-1.json"), r#"{"answer":"sqlite"}"#).unwrap();
    let r = result(p);
    assert_eq!(r["result"]["answer"], "sqlite", "{r}");
}

#[test]
fn a2_timeout_is_a_clean_error() {
    let d = tempfile::tempdir().unwrap();
    let t = Instant::now();
    let r = result(spawn(d.path(), "1", serde_json::json!({"question":"anyone?"})));
    assert!(t.elapsed() < Duration::from_secs(5));
    assert!(r["error"].as_str().unwrap().contains("no answer"), "{r}");
}

#[test]
fn a3_requires_a_question_and_is_wired() {
    let d = tempfile::tempdir().unwrap();
    let r = result(spawn(d.path(), "1", serde_json::json!({})));
    assert!(r["error"].as_str().unwrap().contains("question"), "{r}");
    let t = hs_loop::toolschema::schema_for("ask_user_question", "apply").expect("schema");
    assert_eq!(t["function"]["parameters"]["required"][0], "question");
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    assert!(std::fs::read_to_string(root.join("install.sh")).unwrap().contains("hs-plugin-askuser"));
    assert!(std::fs::read_to_string(root.join("hairspring.example.toml")).unwrap().contains("name = \"ask_user_question\""));
}
