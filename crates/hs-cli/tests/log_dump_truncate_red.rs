//! RED (hostile review, 2026-09-09): `hs-log-cli dump --payloads` claims to
//! cap printed payloads and mark them truncated, but the cap is inert:
//! `if t.len() > 3000 { t.truncate(2_000_000) }` shortens nothing yet still
//! appends the "[truncated]" marker, so oversize payloads print in full
//! under a false label. Both dump and trace also truncate Strings at fixed
//! byte offsets, which panics when the offset splits a multi-byte char.
//!
//! Falsifiers: a >3000-byte payload must not appear whole in dump output
//! while the marker must be present; a payload or completion whose cut
//! point splits a multi-byte char must exit successfully, not panic.

use hs_core::*;
use hs_log::*;
use std::process::{Command, Stdio};
use uuid::Uuid;

fn seed(dir: &std::path::Path, kind: EventKind, payload: Vec<u8>) {
    std::fs::create_dir_all(dir).unwrap();
    let mut w = StreamWriter::create(dir, Uuid::new_v4()).unwrap();
    w.append(EventBuilder::new(kind).payload(Payload::Inline(payload)))
        .unwrap();
}

fn run(dir: &std::path::Path, args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_hs-log-cli"))
        .args(args)
        .arg("--dir")
        .arg(dir.to_str().unwrap())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .unwrap()
}

#[test]
fn dump_payloads_actually_truncates_oversize_payloads() {
    let tmp = tempfile::tempdir().unwrap();
    seed(tmp.path(), EventKind::ToolCall, vec![b'A'; 5000]);
    let out = run(tmp.path(), &["dump", "--payloads"]);
    assert!(
        out.status.success(),
        "dump failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let s = String::from_utf8_lossy(&out.stdout);
    assert!(
        !s.contains(&"A".repeat(4000)),
        "payload printed in full despite the truncation marker"
    );
    assert!(s.contains("[truncated]"), "missing truncation marker:\n{s}");
}

#[test]
fn dump_payloads_truncation_respects_char_boundaries() {
    let tmp = tempfile::tempdir().unwrap();
    // "A" + 1500 x U+20AC = 1 + 4500 bytes; boundaries at 1+3k, so the
    // 3000-byte cut falls mid-char. Must not panic.
    seed(
        tmp.path(),
        EventKind::ToolCall,
        format!("A{}", "\u{20ac}".repeat(1500)).into_bytes(),
    );
    let out = run(tmp.path(), &["dump", "--payloads"]);
    assert!(
        out.status.success(),
        "dump panicked on a char boundary: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let s = String::from_utf8_lossy(&out.stdout);
    assert!(s.contains("[truncated]"), "missing truncation marker:\n{s}");
}

#[test]
fn trace_truncation_respects_char_boundaries() {
    let tmp = tempfile::tempdir().unwrap();
    // Completion "A" + 150 x U+20AC = 451 bytes; boundaries at 1+3k, so
    // the 300-byte cut falls mid-char.
    let payload = serde_json::to_vec(&serde_json::json!({
        "model": "m",
        "completion": format!("A{}", "\u{20ac}".repeat(150)),
        "reasoning_tokens": 1,
        "reasoning_content": "r",
    }))
    .unwrap();
    seed(tmp.path(), EventKind::ModelCall, payload);
    let out = run(tmp.path(), &["trace"]);
    assert!(
        out.status.success(),
        "trace panicked on a char boundary: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let s = String::from_utf8_lossy(&out.stdout);
    assert!(s.contains("[truncated]"), "missing truncation marker:\n{s}");
}
