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
