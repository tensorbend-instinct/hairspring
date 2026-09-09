//! B4 RED (v5 spec 2.6/2.7/9.11, checklist items 1.6+2.6): capability
//! swaps are log transactions, and one stream has ONE live writer.
//!
//! c1: the REPL/TUI model picker (`set_model_override`, the exact path
//!     `UiCmd::SetModel` drives) must book a `capability_change` event on
//!     the session stream naming the old and new bindings. Today the
//!     override flips an in-memory field and the log stays silent:
//!     spec cut #11 "No silent capability changes" violated.
//! c2: at most one executor holds continuation authority over a stream
//!     (spec 2.6). Resuming a stream that a live session already holds
//!     must be REJECTED; today both writers attach and interleave,
//!     corrupting the seq/hash chain by construction.

use hs_loop::repl::{load_session, ReplSession};

fn write_fixture(dir: &std::path::Path) {
    std::fs::create_dir_all(dir).unwrap();
    let toml = r#"
[[tools]]
name = "answer.submit"
command = ["/mnt/instinct-nvme/hairspring/target/debug/hs-plugin-answersubmit"]
subjects = ["*"]

[[tools]]
name = "checker.run"
command = ["/mnt/instinct-nvme/hairspring/target/debug/hs-plugin-liechecker"]
subjects = ["*"]

[[models]]
name = "scripted"
command = ["/mnt/instinct-nvme/hairspring/target/debug/hs-plugin-scripted"]
default = true
subjects = ["*"]

[[models]]
name = "scripted-b"
command = ["/bin/sh", "-c", "HS_SCRIPTED_NAME=scripted-b exec /mnt/instinct-nvme/hairspring/target/debug/hs-plugin-scripted"]
subjects = ["*"]
"#;
    std::fs::write(dir.join("hairspring.toml"), toml).unwrap();
    // One sacrificial line: the kernel probes the model plugin at load.
    std::fs::write(
        dir.join("script.jsonl"),
        "{\"tool\":\"answer.submit\",\"args\":{\"path\":\"probe\",\"content\":\"probe\"}}\n",
    )
    .unwrap();
}

fn stream_ledger(log_root: &std::path::Path, sid: uuid::Uuid) -> String {
    let reader = hs_log::StreamReader::open(log_root, sid).unwrap();
    let mut all = String::new();
    for ev in reader.events().unwrap() {
        all.push_str(&format!("{:?} ", ev.kind));
        if let Ok(b) = reader.resolve_payload(&ev) {
            all.push_str(&String::from_utf8_lossy(&b));
        }
        all.push('\n');
    }
    all
}

#[test]
fn c1_model_picker_books_capability_change() {
    // FIXME: Audit that the environment access only happens in single-threaded code.
    unsafe { std::env::set_var("HS_ANSWER_RAW", "1") };
    // FIXME: Audit that the environment access only happens in single-threaded code.
    unsafe { std::env::set_var("HS_SCRIPTED_PROMPT_AWARE", "1") };
    let dir = std::env::temp_dir().join("cap-change-c1");
    let _ = std::fs::remove_dir_all(&dir);
    write_fixture(&dir);
    // FIXME: Audit that the environment access only happens in single-threaded code.
    unsafe { std::env::set_var("HS_SEQMODEL_SCRIPT", dir.join("script.jsonl")) };
    let run = dir.join("run");
    let mut s = load_session(&dir.join("hairspring.toml"), &run, false, 8, None, None).unwrap();
    let sid = s.vitals().stream_id;
    // The TUI Models picker path: UiCmd::SetModel -> set_model_override.
    s.set_model_override(Some("scripted-b".to_string())).unwrap();
    let ledger = stream_ledger(&run, sid);
    assert!(
        ledger.contains("CapabilityChange"),
        "model swap books a capability_change event (spec 2.7/9.11); ledger:\n{ledger}"
    );
    assert!(
        ledger.contains("scripted") && ledger.contains("scripted-b"),
        "the event names old and new bindings; ledger:\n{ledger}"
    );
}

#[test]
fn c2_one_live_writer_per_stream() {
    // FIXME: Audit that the environment access only happens in single-threaded code.
    unsafe { std::env::set_var("HS_ANSWER_RAW", "1") };
    // FIXME: Audit that the environment access only happens in single-threaded code.
    unsafe { std::env::set_var("HS_SCRIPTED_PROMPT_AWARE", "1") };
    let dir = std::env::temp_dir().join("cap-change-c2");
    let _ = std::fs::remove_dir_all(&dir);
    write_fixture(&dir);
    // FIXME: Audit that the environment access only happens in single-threaded code.
    unsafe { std::env::set_var("HS_SEQMODEL_SCRIPT", dir.join("script.jsonl")) };
    let run = dir.join("run");
    let a = load_session(&dir.join("hairspring.toml"), &run, false, 8, None, None).unwrap();
    let sid = a.vitals().stream_id;
    // A second REPL resuming the stream A still holds must be fenced off
    // (spec 2.6 single-authority fencing). A dropped writer frees it.
    let second = ReplSession::load_resume(&dir.join("hairspring.toml"), &run, false, 8, sid);
    assert!(
        second.is_err(),
        "resuming a stream with a live writer is rejected (spec 2.6 fencing)"
    );
    let err = second.err().unwrap().to_string();
    assert!(
        err.contains("held") || err.contains("locked") || err.contains("fenc"),
        "the rejection says why: {err}"
    );
    drop(a);
    let third = ReplSession::load_resume(&dir.join("hairspring.toml"), &run, false, 8, sid);
    assert!(
        third.is_ok(),
        "after the holder drops, the stream can be resumed: {:?}",
        third.err()
    );
}
