//! RED (hostile review, 2026-09-09): the D1 context assembler truncates
//! tool-result text with `String::truncate` at raw byte caps
//! (`LINE_CAP/CONTENT_CAP` = `20_000`) - a multi-byte UTF-8 char straddling
//! the cut panics the whole mission loop. Same class as `utf8_tail_red`.
//!
//! Falsifier: a `ToolCall` payload whose result places 'é' across byte
//! `20_000` of the assembled line/content must truncate, not panic.

use hs_core::{EventBuilder, EventKind, Payload};

fn big_result_payload(prefix_len: usize) -> Vec<u8> {
    // Content shaped so the cut lands mid-'é' no matter the exact prefix:
    // run of x's ending 1 byte before the cap, then 20 é's around it.
    let content = format!(
        "{}{}{}",
        "x".repeat(20_000 - prefix_len - 1),
        "é".repeat(20),
        "x".repeat(100)
    );
    serde_json::json!({"plugin": "p", "args": null, "result": content})
        .to_string()
        .into_bytes()
}

fn rig(prefix_len: usize) -> (tempfile::TempDir, hs_log::StreamReader, Vec<hs_core::Event>) {
    let dir = tempfile::tempdir().unwrap();
    let stream = uuid::Uuid::new_v4();
    let mut log = hs_log::StreamWriter::create(dir.path(), stream).unwrap();
    log.append(
        EventBuilder::new(EventKind::ToolCall)
            .payload(Payload::Inline(big_result_payload(prefix_len))),
    )
    .unwrap();
    let reader = hs_log::StreamReader::open(dir.path(), stream).unwrap();
    let events = reader.events().unwrap();
    (dir, reader, events)
}

#[test]
fn assemble_truncates_multibyte_line_without_panicking() {
    // assemble() line = "p(null) => " + result: prefix is 11 bytes.
    let (_d, reader, events) = rig(12); // "p(null) => " + opening quote = 12; cut lands mid-first-é
    let a = hs_loop::assembler::assemble(&reader, &events, 1_000_000);
    assert!(
        a.entries[0].contains("...[truncated]"),
        "over-cap line must be marked truncated"
    );
}

#[test]
fn assemble_messages_truncates_multibyte_content_without_panicking() {
    // assemble_messages() truncates the raw content before formatting:
    // prefix is 0.
    let (_d, reader, events) = rig(0);
    let a = hs_loop::assembler::assemble_messages(&reader, &events, 1_000_000);
    let tool_msg = &a.messages[1];
    assert!(
        tool_msg["content"].as_str().unwrap().contains("...[truncated]"),
        "over-cap content must be marked truncated"
    );
}
