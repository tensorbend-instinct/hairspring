//! D1 token-budgeted context assembler (replaces the 60KB char window).
//! The transcript is a PROJECTION of the stream's own ToolCall events:
//! everything that fits the budget stays verbatim; only over-budget missions
//! compress, and compression is a pointer into the always-resident LEDGER
//! plus the source event range (audit refs), never a bare tally.

use hs_core::EventKind;

pub struct Compressed {
    pub count: usize,
    pub lo_seq: u64,
    pub hi_seq: u64,
    pub lo_id: uuid::Uuid,
    pub hi_id: uuid::Uuid,
    /// The verbatim lines being compressed - input to the LLM distiller.
    pub lines: Vec<String>,
}

pub struct Assembly {
    pub entries: Vec<String>,
    pub compressed: Option<Compressed>,
}

/// Build the transcript block from the stream's ToolCall events.
/// `budget_chars` = context_budget_tokens * 4 (the loop owns the token config).
pub fn assemble(reader: &hs_log::StreamReader, events: &[hs_core::Event], budget_chars: usize) -> Assembly {
    const LINE_CAP: usize = 20_000;
    let mut lines: Vec<(u64, uuid::Uuid, String)> = vec![];
    for e in events.iter().rev() {
        if e.kind != EventKind::ToolCall {
            continue;
        }
        // resolve Inline AND BlobRef payloads: hs-log promotes large results
        // to blob refs on append; skipping them drops big tool outputs
        if let Ok(bytes) = reader.resolve_payload(e) {
            if let Ok(v) = serde_json::from_slice::<serde_json::Value>(&bytes) {
                let mut line = format!(
                    "{}({}) => {}",
                    v["plugin"].as_str().unwrap_or("?"),
                    v["args"],
                    v["result"]
                );
                if line.len() > LINE_CAP {
                    line.truncate(LINE_CAP);
                    line.push_str("...[truncated]");
                }
                lines.push((e.seq, e.event_id, line));
            }
        }
    }
    let total: usize = lines.iter().map(|(_, _, l)| l.len() + 8).sum();
    let mut entries: Vec<String> = vec![];
    if total <= budget_chars {
        // everything fits: all verbatim, newest-first fill (never breaks)
        let mut budget = budget_chars;
        for (_, _, line) in lines {
            if line.len() + 8 > budget {
                break;
            }
            budget -= line.len() + 8;
            entries.push(line);
        }
        entries.reverse();
        return Assembly { entries, compressed: None };
    }
    // over budget: recent tail verbatim (60% of budget), oldest compressed
    // into a ledger pointer with audit refs - content-preserving via D2
    let mut budget = budget_chars * 60 / 100;
    let mut kept: Vec<String> = vec![];
    let mut compacted: Vec<(u64, uuid::Uuid, String)> = vec![];
    for (seq, id, line) in lines {
        if compacted.is_empty() && line.len() + 8 <= budget {
            budget -= line.len() + 8;
            kept.push(line);
        } else {
            compacted.push((seq, id, line));
        }
    }
    let mut compressed = None;
    if !compacted.is_empty() {
        let lo = compacted.last().unwrap();
        let hi = compacted.first().unwrap();
        entries.push(format!(
            "COMPACTED {} earlier tool calls (events seq {}..{}, refs {}..{}): distilled into the LEDGER block above - reads, edits, and test verdicts from that range are recorded there",
            compacted.len(), lo.0, hi.0, lo.1, hi.1
        ));
        compressed = Some(Compressed {
            count: compacted.len(),
            lo_seq: lo.0,
            hi_seq: hi.0,
            lo_id: lo.1,
            hi_id: hi.1,
            lines: compacted.iter().map(|(_, _, l)| l.clone()).collect(),
        });
    }
    kept.reverse();
    entries.extend(kept);
    Assembly { entries, compressed }
}

/// Native-messages variant of the transcript projection (structured-
/// messages migration 2026-09-05): the same log-sourced, token-budgeted
/// projection as assemble(), but each exchange is emitted as a native
/// assistant(tool_calls) + tool pair, and over-budget compaction is a user
/// handoff message. No hand-rendered transcript text survives.
pub struct AssemblyMessages {
    pub messages: Vec<serde_json::Value>,
    pub compressed: Option<Compressed>,
}

struct Exchange {
    seq: u64,
    id: uuid::Uuid,
    plugin: String,
    args: serde_json::Value,
    content: String,
    line: String,
    /// The owning ModelCall's reasoning_content (thinking-mode pass-back;
    /// empty when unrecorded).
    reasoning: String,
}

/// Build the history messages from the stream's ToolCall events.
/// `budget_chars` = context_budget_tokens * 4 (the loop owns the config).
pub fn assemble_messages(
    reader: &hs_log::StreamReader,
    events: &[hs_core::Event],
    budget_chars: usize,
) -> AssemblyMessages {
    const CONTENT_CAP: usize = 20_000;
    const PAIR_OVERHEAD: usize = 64; // role/id/type framing, chars
    // Chronological pre-pass: pair each ToolCall with the reasoning of
    // the ModelCall that produced it (nearest preceding ModelCall - the
    // one-tool-call-per-reply protocol makes that exact).
    let mut reasoning_by_tc: std::collections::HashMap<u64, String> =
        std::collections::HashMap::new();
    let mut last_reasoning = String::new();
    for e in events.iter() {
        match e.kind {
            EventKind::ModelCall => {
                if let Ok(bytes) = reader.resolve_payload(e) {
                    if let Ok(v) = serde_json::from_slice::<serde_json::Value>(&bytes) {
                        last_reasoning =
                            v["reasoning_content"].as_str().unwrap_or("").to_string();
                    }
                }
            }
            EventKind::ToolCall => {
                if !last_reasoning.is_empty() {
                    reasoning_by_tc.insert(e.seq, last_reasoning.clone());
                }
            }
            _ => {}
        }
    }
    let mut exch: Vec<Exchange> = vec![];
    for e in events.iter().rev() {
        if e.kind != EventKind::ToolCall {
            continue;
        }
        // resolve Inline AND BlobRef payloads, same as assemble()
        if let Ok(bytes) = reader.resolve_payload(e) {
            if let Ok(v) = serde_json::from_slice::<serde_json::Value>(&bytes) {
                let plugin = v["plugin"].as_str().unwrap_or("?").to_string();
                let args = v["args"].clone();
                let mut content = if !v["result"].is_null() {
                    match v["result"].as_str() {
                        Some(s) => s.to_string(),
                        None => v["result"].to_string(),
                    }
                } else if !v["error"].is_null() {
                    format!("error: {}", v["error"].as_str().unwrap_or("?"))
                } else {
                    "null".to_string()
                };
                if content.len() > CONTENT_CAP {
                    content.truncate(CONTENT_CAP);
                    content.push_str("...[truncated]");
                }
                let line = format!("{}({}) => {}", plugin, v["args"], content);
                let reasoning = reasoning_by_tc.get(&e.seq).cloned().unwrap_or_default();
                exch.push(Exchange { seq: e.seq, id: e.event_id, plugin, args, content, line, reasoning });
            }
        }
    }
    let cost =
        |x: &Exchange| x.args.to_string().len() + x.content.len() + x.reasoning.len() + PAIR_OVERHEAD;
    let total: usize = exch.iter().map(&cost).sum();
    let mut messages: Vec<serde_json::Value> = vec![];
    if total <= budget_chars {
        // everything fits: newest-first fill, then back to chronological
        let mut budget = budget_chars;
        let mut kept: Vec<&Exchange> = vec![];
        for x in &exch {
            if cost(x) > budget {
                break;
            }
            budget -= cost(x);
            kept.push(x);
        }
        kept.reverse();
        for x in kept {
            let (a, t) =
                crate::msgfmt::exchange_pair(x.seq, &x.plugin, &x.args, &x.content, &x.reasoning);
            messages.push(a);
            messages.push(t);
        }
        return AssemblyMessages { messages, compressed: None };
    }
    // over budget: recent tail verbatim (60%), oldest into a compaction
    // handoff message with audit refs (content-preserving via D2)
    let mut budget = budget_chars * 60 / 100;
    let mut kept: Vec<&Exchange> = vec![];
    let mut compacted: Vec<&Exchange> = vec![];
    for x in &exch {
        if compacted.is_empty() && cost(x) <= budget {
            budget -= cost(x);
            kept.push(x);
        } else {
            compacted.push(x);
        }
    }
    let mut compressed = None;
    if !compacted.is_empty() {
        let lo = compacted.last().unwrap();
        let hi = compacted.first().unwrap();
        messages.push(serde_json::json!({
            "role": "user",
            "content": format!(
                "COMPACTED {} earlier tool calls (events seq {}..{}, refs {}..{}): distilled into the LEDGER block of the state tail - reads, edits, and test verdicts from that range are recorded there",
                compacted.len(), lo.seq, hi.seq, lo.id, hi.id
            ),
        }));
        compressed = Some(Compressed {
            count: compacted.len(),
            lo_seq: lo.seq,
            hi_seq: hi.seq,
            lo_id: lo.id,
            hi_id: hi.id,
            lines: compacted.iter().rev().map(|x| x.line.clone()).collect(),
        });
    }
    kept.reverse();
    for x in kept {
        let (a, t) =
            crate::msgfmt::exchange_pair(x.seq, &x.plugin, &x.args, &x.content, &x.reasoning);
        messages.push(a);
        messages.push(t);
    }
    AssemblyMessages { messages, compressed }
}
