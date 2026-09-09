//! Post-mission extraction (openJiuwen Self-Reflection analog): distill a
//! stream's trajectory into typed records. Deterministic rules today; the
//! LLM distiller (Codex four-element handoff contract) plugs in here later
//! and writes through the same provenance contract.

use crate::NewMemoryRecord;
use hs_core::EventKind;
use std::path::Path;

#[must_use]
pub fn extract_stream(
    log_root: &Path,
    stream_id: uuid::Uuid,
    mission_id: &str,
    agent_id: &str,
) -> Vec<NewMemoryRecord> {
    let Ok(reader) = hs_log::StreamReader::open(log_root, stream_id) else {
        return vec![];
    };
    let Ok(events) = reader.events() else {
        return vec![];
    };
    let mut edits: Vec<String> = vec![];
    let mut edit_seqs: Vec<u64> = vec![];
    let mut failures: Vec<(u64, String)> = vec![];
    let mut verdict_seqs: Vec<u64> = vec![];
    let mut passed = false;
    let mut tool_calls = 0u32;
    for e in &events {
        let Ok(bytes) = reader.resolve_payload(e) else {
            continue;
        };
        let Ok(v) = serde_json::from_slice::<serde_json::Value>(&bytes) else {
            continue;
        };
        match e.kind {
            EventKind::ToolCall => {
                tool_calls += 1;
                if v["plugin"].as_str() == Some("answer.write")
                    && let Some(p) = v["args"]["path"].as_str() {
                        edits.push(p.to_string());
                        edit_seqs.push(e.seq);
                    }
            }
            EventKind::Feedback if v["checker"].is_string() => {
                verdict_seqs.push(e.seq);
                if v["passed"].as_bool() == Some(true) {
                    passed = true;
                } else {
                    let err = v["error"].as_str().unwrap_or("unknown").to_string();
                    if !err.is_empty() {
                        failures.push((e.seq, err));
                    }
                }
            }
            _ => {}
        }
    }
    if tool_calls == 0 {
        return vec![];
    }
    let mut out = vec![];
    let mut seqs = edit_seqs.clone();
    seqs.extend(verdict_seqs.iter());
    seqs.sort_unstable();
    let edits_s = if edits.is_empty() {
        "none".into()
    } else {
        edits.join(", ")
    };
    let fails_s = if failures.is_empty() {
        "none".into()
    } else {
        failures
            .iter()
            .map(|(_, e)| e.as_str())
            .collect::<Vec<_>>()
            .join(" | ")
    };
    out.push(NewMemoryRecord {
        agent_id: agent_id.to_string(),
        mission_id: Some(mission_id.to_string()),
        kind: "episodic".into(),
        content: format!(
            "mission {mission_id} ended {} after {tool_calls} tool calls; edits: {edits_s}; failing checks: {fails_s}",
            if passed { "PASS" } else { "FAIL" }
        ),
        importance: 0.6,
        expires_at: None,
        source_seqs: seqs,
    });
    for (seq, err) in failures {
        out.push(NewMemoryRecord {
            agent_id: agent_id.to_string(),
            mission_id: Some(mission_id.to_string()),
            kind: "procedural".into(),
            content: format!("in mission {mission_id}, checker.run FAILED: {err}"),
            importance: 0.7,
            expires_at: None,
            source_seqs: vec![seq],
        });
    }
    out
}
