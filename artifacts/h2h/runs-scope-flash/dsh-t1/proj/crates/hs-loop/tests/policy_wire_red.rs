//! RED (hostile review, 2026-09-09): `hs-plugin-policy`'s dispatch matches
//! only the literal method "`policy.propose_prompt`" - but the kernel's
//! `ToolCall` path ALWAYS sends method "tool.call" (hs-kernel dispatch). So a
//! model invoking the policy tool in a live mission gets
//! "$error: unknown method" every time: the spec-gate-8 self-instruction
//! proposal path is dead over the wire. Every sibling tool plugin matches
//! "tool.call".
//!
//! Falsifier: driving the compiled plugin binary with a tool.call frame
//! must record the proposal, not error.

use std::io::{BufRead, BufReader, Write};

const POLICY: &str = env!("CARGO_BIN_EXE_hs-plugin-policy");

#[test]
fn policy_tool_answers_tool_call_frames() {
    let dir = tempfile::tempdir().unwrap();
    let mut child = std::process::Command::new(POLICY)
        .env("HS_RUN_DIR", dir.path())
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    let mut stdin = child.stdin.take().unwrap();
    let mut stdout = BufReader::new(child.stdout.take().unwrap());
    writeln!(
        stdin,
        "{}",
        serde_json::json!({"id": 1, "method": "tool.call",
            "params": {"args": {"name": "swe-mission", "text": "always repo.exec first"}}})
    )
    .unwrap();
    stdin.flush().unwrap();
    let mut line = String::new();
    stdout.read_line(&mut line).unwrap();
    let resp: serde_json::Value = serde_json::from_str(&line).unwrap();
    assert!(
        resp.get("error").is_none(),
        "tool.call frame must not error: {resp}"
    );
    assert_eq!(resp["result"]["recorded"], serde_json::json!(true));
    drop(stdin);
    let _ = child.wait();
}
