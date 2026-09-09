//! RED (Eric 2026-09-06): reasoning traces are recorded in every `ModelCall`
//! event payload but have no live-readable form. The harness needs to FOLLOW
//! the model's thinking as missions run, not only after.
//!
//! hs-log-cli gains:  trace --dir D            replay every `ModelCall`
//!                                             reasoning trace in seq order
//!                    trace --dir D --follow   keep emitting as new events land
//!
//! Falsifiers: replay must print both seeded traces in order and must not
//! emit non-ModelCall events; follow must emit a trace appended AFTER the
//! watcher started, within a bounded wait.

use hs_core::*;
use hs_log::*;
use std::io::{BufRead, BufReader};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};
use uuid::Uuid;

fn model_payload(reasoning: &str, completion: &str) -> Vec<u8> {
    serde_json::to_vec(&serde_json::json!({
        "model": "deepseek-v4-pro",
        "completion": completion,
        "reasoning_tokens": 42,
        "reasoning_content": reasoning,
    }))
    .unwrap()
}

fn seed_log(dir: &std::path::Path) -> Uuid {
    std::fs::create_dir_all(dir).unwrap();
    let stream = Uuid::new_v4();
    let mut w = StreamWriter::create(dir, stream).unwrap();
    w.append(
        EventBuilder::new(EventKind::ModelCall).payload(Payload::Inline(model_payload(
            "thinking about step one",
            "{\"tool\":\"repo.search\"}",
        ))),
    )
    .unwrap();
    w.append(
        EventBuilder::new(EventKind::ToolCall).payload(Payload::Inline(
            serde_json::to_vec(&serde_json::json!({"tool": "repo.search", "args": {}})).unwrap(),
        )),
    )
    .unwrap();
    w.append(
        EventBuilder::new(EventKind::ModelCall).payload(Payload::Inline(model_payload(
            "thinking about step two",
            "{\"tool\":\"repo.read\"}",
        ))),
    )
    .unwrap();
    stream
}

#[test]
fn trace_replay_prints_reasoning_in_order_and_skips_non_model_events() {
    let tmp = tempfile::tempdir().unwrap();
    seed_log(tmp.path());
    let out = Command::new(env!("CARGO_BIN_EXE_hs-log-cli"))
        .args(["trace", "--dir", tmp.path().to_str().unwrap()])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "trace replay exited {:?}: {}",
        out.status.code(),
        String::from_utf8_lossy(&out.stderr)
    );
    let s = String::from_utf8_lossy(&out.stdout);
    let one = s
        .find("thinking about step one")
        .unwrap_or_else(|| panic!("missing trace one in:\n{s}"));
    let two = s
        .find("thinking about step two")
        .unwrap_or_else(|| panic!("missing trace two in:\n{s}"));
    assert!(one < two, "traces out of order:\n{s}");
    // Exactly two trace bodies: the ToolCall event must not be printed as a trace.
    assert_eq!(
        s.matches("THINK:").count(),
        2,
        "expected exactly 2 THINK blocks (ModelCall only):\n{s}"
    );
}

#[test]
fn trace_follow_emits_events_appended_after_start() {
    let tmp = tempfile::tempdir().unwrap();
    let stream = seed_log(tmp.path());
    let mut child = Command::new(env!("CARGO_BIN_EXE_hs-log-cli"))
        .args(["trace", "--dir", tmp.path().to_str().unwrap(), "--follow"])
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let stdout = child.stdout.take().unwrap();
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        for line in BufReader::new(stdout).lines() {
            if tx.send(line.unwrap_or_default()).is_err() {
                return;
            }
        }
    });
    // Drain the replayed backlog first, then append a NEW event.
    let deadline = Instant::now() + Duration::from_secs(5);
    let mut saw_two = false;
    while Instant::now() < deadline && !saw_two {
        if let Ok(l) = rx.recv_timeout(Duration::from_millis(200)) {
            saw_two = saw_two || l.contains("thinking about step two");
        }
    }
    assert!(saw_two, "follow mode never replayed the seeded backlog");
    {
        let mut w = StreamWriter::resume(tmp.path(), stream).unwrap().writer;
        w.append(
            EventBuilder::new(EventKind::ModelCall).payload(Payload::Inline(model_payload(
                "thinking about step three",
                "{\"tool\":\"submit\"}",
            ))),
        )
        .unwrap();
    }
    let deadline = Instant::now() + Duration::from_secs(5);
    let mut saw_three = false;
    while Instant::now() < deadline && !saw_three {
        if let Ok(l) = rx.recv_timeout(Duration::from_millis(200)) {
            saw_three = saw_three || l.contains("thinking about step three");
        }
    }
    let _ = child.kill();
    let _ = child.wait();
    assert!(
        saw_three,
        "follow mode did not emit the post-start trace within 5s"
    );
}
