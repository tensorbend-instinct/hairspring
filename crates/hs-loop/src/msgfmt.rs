//! Structured-messages formatting (user directive 2026-09-05: "EVERYTHING
//! native, transcript included"). The operator model call is a native chat
//! messages array; this module owns the two cross-cutting needs that
//! creates: building history pairs, and giving scripted FIXTURE models a
//! faithful single-text view of what the model sees (fixtures are test
//! doubles that pattern-match prompt text; real providers always receive
//! the structured array verbatim).

use serde_json::Value;


/// Last `max_bytes` of `s`, advanced to a UTF-8 char boundary. Slicing a
/// String at a raw byte offset panics when the offset splits a multi-byte
/// char, and command/model output is arbitrary UTF-8 (RED `utf8_tail_red`,
/// 2026-09-09: termexec, selfcheck and repexec all panicked on
/// `'\u{e9}'*k + "x"` output).
#[must_use]
pub fn tail_bytes_safe(s: &str, max_bytes: usize) -> String {
    if s.len() <= max_bytes {
        return s.to_string();
    }
    let mut cut = s.len() - max_bytes;
    while !s.is_char_boundary(cut) {
        cut += 1;
    }
    s[cut..].to_string()
}

/// First `max_bytes` of `s`, backed off to a UTF-8 char boundary.
#[must_use]
pub fn prefix_bytes_safe(s: &str, max_bytes: usize) -> String {
    if s.len() <= max_bytes {
        return s.to_string();
    }
    let mut cut = max_bytes;
    while !s.is_char_boundary(cut) {
        cut -= 1;
    }
    s[..cut].to_string()
}

/// One history exchange as a native `assistant(tool_calls)` + tool pair.
/// The `tool_call` id is deterministic from the owning event's seq, so the
/// replayed prefix is byte-identical across steps (the KV-cache contract).
/// `reasoning` is the owning `ModelCall`'s `reasoning_content`: thinking-mode
/// providers with a `tools` parameter REQUIRE it passed back in later
/// turns (`DeepSeek` V4: 400 otherwise). Empty reasoning omits the field,
/// keeping legacy/scripted replays byte-identical.
#[must_use]
pub fn exchange_pair(
    seq: u64,
    plugin: &str,
    args: &Value,
    content: &str,
    reasoning: &str,
) -> (Value, Value) {
    let id = format!("call_{seq}");
    let args_str = if args.is_null() {
        "{}".to_string()
    } else {
        args.to_string()
    };
    (
        if reasoning.is_empty() {
            serde_json::json!({
                "role": "assistant",
                "tool_calls": [{
                    "id": id,
                    "type": "function",
                    "function": {
                        "name": crate::toolschema::wire_name(plugin),
                        "arguments": args_str,
                    }
                }]
            })
        } else {
            serde_json::json!({
                "role": "assistant",
                "reasoning_content": reasoning,
                "tool_calls": [{
                    "id": id,
                    "type": "function",
                    "function": {
                        "name": crate::toolschema::wire_name(plugin),
                        "arguments": args_str,
                    }
                }]
            })
        },
        serde_json::json!({
            "role": "tool",
            "tool_call_id": id,
            "content": content,
        }),
    )
}

/// Flatten a model.call params value into the single-text view a scripted
/// fixture matches on. Legacy callers (distill, one-shot audits) pass
/// "prompt" and get it back untouched; operator calls pass "messages" and
/// get the faithful rendering: message contents verbatim, history pairs as
/// "- tool(args) => result" lines. Every marker the fixtures key on
/// (`ANSWER_PATH`:, FEEDBACK:, MARKER-*, COMPACTED, DISTILL:) survives.
#[must_use]
pub fn prompt_view(params: &Value) -> String {
    if let Some(p) = params["prompt"].as_str() {
        return p.to_string();
    }
    let mut out = String::new();
    let Some(msgs) = params["messages"].as_array() else {
        return out;
    };
    for m in msgs {
        match m["role"].as_str().unwrap_or("") {
            "assistant" => {
                if let Some(tcs) = m["tool_calls"].as_array() {
                    for tc in tcs {
                        let name = crate::toolschema::internal_name(
                            tc["function"]["name"].as_str().unwrap_or("?"),
                        );
                        let args = tc["function"]["arguments"].as_str().unwrap_or("{}");
                        out.push_str(&format!("- {name}({args})"));
                    }
                }
            }
            "tool" => {
                out.push_str(&format!(" => {}\n", m["content"].as_str().unwrap_or("")));
            }
            _ => {
                if let Some(c) = m["content"].as_str() {
                    out.push_str(c);
                    out.push('\n');
                }
            }
        }
    }
    out
}
