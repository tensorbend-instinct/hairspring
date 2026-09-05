//! RED: reasoning_content pass-back in replayed history (DeepSeek V4
//! thinking+tools contract, 2026-09-05; api-docs.deepseek.com/guides/
//! thinking_mode: when the request carries `tools`, every intermediate
//! assistant message's reasoning_content MUST be passed back in later
//! turns, or the API returns 400). The assembler replays log history as
//! assistant(tool_calls) + tool pairs; each pair must carry the
//! reasoning_content of the ModelCall that produced it (recorded on the
//! ModelCall payload since b6f65961). A ToolCall with no preceding
//! ModelCall reasoning (scripted fixtures, pre-logging runs) replays
//! byte-identical to before - no field, no filler.

use hs_core::{EventBuilder, EventKind, Payload};
use hs_loop::*;

fn stream_with(
    log: &std::path::Path,
    stream: uuid::Uuid,
    events: &[(EventKind, serde_json::Value)],
) {
    let mut w = hs_log::StreamWriter::create(log, stream).unwrap();
    for (kind, payload) in events {
        w.append(
            EventBuilder::new(*kind).payload(Payload::Inline(
                serde_json::to_vec(payload).unwrap(),
            )),
        )
        .unwrap();
    }
}

#[test]
fn exchange_pair_carries_reasoning_when_present() {
    let (a, _t) = msgfmt::exchange_pair(
        7,
        "repo.read",
        &serde_json::json!({"path": "x"}),
        "contents",
        "the file holds the answer",
    );
    assert_eq!(
        a["reasoning_content"].as_str().unwrap_or(""),
        "the file holds the answer",
        "assistant pair carries the reasoning: {a}"
    );
    assert_eq!(a["tool_calls"][0]["id"], "call_7", "id contract unchanged");
}

#[test]
fn exchange_pair_omits_reasoning_when_empty() {
    let (a, _t) = msgfmt::exchange_pair(
        7,
        "repo.read",
        &serde_json::json!({"path": "x"}),
        "contents",
        "",
    );
    assert!(
        a.get("reasoning_content").is_none(),
        "no reasoning -> no field (byte-identical legacy replay): {a}"
    );
}

#[test]
fn assemble_pairs_reasoning_from_owning_modelcall() {
    let dir = tempfile::tempdir().unwrap();
    let stream = uuid::Uuid::new_v4();
    stream_with(
        dir.path(),
        stream,
        &[
            (
                EventKind::ModelCall,
                serde_json::json!({"model": "m", "completion": "{\"tool\":\"repo.read\"}",
                    "reasoning_content": "first I read the config"}),
            ),
            (
                EventKind::ToolCall,
                serde_json::json!({"plugin": "repo.read", "args": {"path": "a"}, "result": "aaa"}),
            ),
            (
                EventKind::ModelCall,
                serde_json::json!({"model": "m", "completion": "{\"tool\":\"repo.read\"}",
                    "reasoning_content": "then I read the impl"}),
            ),
            (
                EventKind::ToolCall,
                serde_json::json!({"plugin": "repo.read", "args": {"path": "b"}, "result": "bbb"}),
            ),
        ],
    );
    let reader = hs_log::StreamReader::open(dir.path(), stream).unwrap();
    let events = reader.events().unwrap();
    let asm = assembler::assemble_messages(&reader, &events, 1_000_000);
    assert_eq!(asm.messages.len(), 4, "2 exchanges = 2 pairs");
    assert_eq!(
        asm.messages[0]["reasoning_content"].as_str().unwrap_or(""),
        "first I read the config",
        "pair 1 gets ModelCall 1's reasoning: {}",
        asm.messages[0]
    );
    assert_eq!(
        asm.messages[2]["reasoning_content"].as_str().unwrap_or(""),
        "then I read the impl",
        "pair 2 gets ModelCall 2's reasoning: {}",
        asm.messages[2]
    );
}

#[test]
fn assemble_orphan_toolcall_has_no_reasoning_field() {
    let dir = tempfile::tempdir().unwrap();
    let stream = uuid::Uuid::new_v4();
    stream_with(
        dir.path(),
        stream,
        &[(
            EventKind::ToolCall,
            serde_json::json!({"plugin": "repo.read", "args": {"path": "a"}, "result": "aaa"}),
        )],
    );
    let reader = hs_log::StreamReader::open(dir.path(), stream).unwrap();
    let events = reader.events().unwrap();
    let asm = assembler::assemble_messages(&reader, &events, 1_000_000);
    assert!(
        asm.messages[0].get("reasoning_content").is_none(),
        "no preceding ModelCall -> no reasoning field: {}",
        asm.messages[0]
    );
}
