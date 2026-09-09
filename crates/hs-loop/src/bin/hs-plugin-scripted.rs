//! Phase 1 test model "seqmodel": scripted completions from a file
//! (HS_SEQMODEL_SCRIPT), one JSON line per model.call, cycling on the last
//! line when the script runs out. This isolates loop behavior (supervisor
//! aborts, progress booking) from any real model.
include!("shared/sdk.rs");

/// The ANSWER_PATH value out of the operator prompt's volatile tail.
fn extract_answer_path(prompt: &str) -> Option<String> {
    let i = prompt.find("ANSWER_PATH: ")?;
    let rest = &prompt[i + 13..];
    let end = rest.find('\n').unwrap_or(rest.len());
    let p = rest[..end].trim();
    if p.is_empty() {
        None
    } else {
        Some(p.to_string())
    }
}

/// Whether the prompt's ARTIFACT block already shows a written
/// answer (header line ends with "):" then the body, until the next
/// section or the prompt tail).
fn artifact_on_disk(prompt: &str) -> bool {
    let Some(i) = prompt.find("ARTIFACT") else {
        return false;
    };
    let after = &prompt[i..];
    let Some(j) = after.find("):\n") else {
        return false;
    };
    let body = &after[j + 3..];
    let end = body
        .find("\nFEEDBACK")
        .or_else(|| body.find("\nLEDGER"))
        .or_else(|| body.find("\nMEMORY"))
        .or_else(|| body.find("\nATTEMPT"))
        .unwrap_or(body.len());
    let b = body[..end].trim();
    // artifact_section renders the empty artifact as "<none>".
    !b.is_empty() && b != "<none>"
}

fn main() {
    let script: Vec<String> = std::fs::read_to_string(std::env::var("HS_SEQMODEL_SCRIPT").unwrap())
        .unwrap()
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| l.to_string())
        .collect();
    assert!(!script.is_empty(), "empty seqmodel script");
    let deltas = std::env::var("HS_SEQMODEL_DELTAS").as_deref() == Ok("1");
    let mut n = 0usize;
    serve_ext(
        "scripted",
        "model",
        &mut move |method, params, emit| match method {
            "model.call" => {
                let __pv;
                let prompt = match params["prompt"].as_str() {
                    Some(p) => p,
                    None => {
                        __pv = hs_loop::msgfmt::prompt_view(&params);
                        __pv.as_str()
                    }
                };
                if prompt.starts_with("DISTILL:") {
                    // distillation calls do not consume the mission script
                    return serde_json::json!({
                        "completion": "PROGRESS AND DECISIONS: read pages 1-3, chose the parser fix\nCONSTRAINTS AND PREFERENCES: no host fs access\nNEXT STEPS: patch parser.rs\nCRITICAL DATA: check.sh is the F2P gate",
                        "input_tokens": prompt.len() / 4 + 1,
                        "output_tokens": 40,
                        "cost_usd_micros": 500
                    });
                }
                // Eric's five #2/#3: the provider is PROMPT-AWARE - it
                // answers what the request asks, not just the next
                // script line. Pre-fix the verifier (offered
                // verdict.submit) got replayed prose -> every green
                // mission booked verifier_malfunction; and an
                // exhausted script replayed its last prose line
                // forever -> every later mission burned to
                // steps_exhausted with no answer ever submitted.
                let offered: Vec<String> = params["tools"]
                    .as_array()
                    .map(|t| {
                        t.iter()
                            .filter_map(|x| {
                                x["function"]["name"].as_str().map(|n| n.replace("__", "."))
                            })
                            .collect()
                    })
                    .unwrap_or_default();
                // Prompt-awareness is OPT-IN (HS_SCRIPTED_PROMPT_AWARE=1):
                // the plugin's original contract - one script line per
                // model.call, cycling on the last - is pinned by
                // verifier_red, structured_messages_red,
                // feedback_integrity_red and repexec_editguard_red, whose
                // setups NEED dumb replay (e.g. prose replies to the
                // verifier audit). The honesty fixture and live TUI proofs
                // opt in.
                let prompt_aware =
                    std::env::var("HS_SCRIPTED_PROMPT_AWARE").as_deref() == Ok("1");
                let completion: String = if !prompt_aware {
                    let c = script[n.min(script.len() - 1)].clone();
                    n += 1;
                    c
                } else if offered.iter().any(|n| n == "verdict.submit") {
                    // An audit of honest scripted work: not refuted.
                    r#"{"tool":"verdict.submit","args":{"refuted":false,"findings":[],"blocking":"none"}}"#.to_string()
                } else if n < script.len() {
                    let c = script[n].clone();
                    n += 1;
                    c
                } else if let Some(path) = extract_answer_path(prompt) {
                    if artifact_on_disk(prompt) {
                        // Answer already written - stand down and let
                        // the checker/verifier run.
                        "The answer is written and ready for grading. ## Done".to_string()
                    } else {
                        // A competent model SUBMITS the answer it was
                        // told to write instead of babbling prose.
                        match offered.iter().find(|n| n.starts_with("answer.")) {
                            Some(t) if t == "answer.submit" => format!(
                                r#"{{"tool":"answer.submit","args":{{"path":"{}","summary":"scripted answer: mission complete"}}}}"#,
                                path
                            ),
                            Some(t) => format!(
                                r#"{{"tool":"{}","args":{{"path":"{}","content":"scripted answer: mission complete"}}}}"#,
                                t, path
                            ),
                            None => "I have no answer tool to submit with. ## Done".to_string(),
                        }
                    }
                } else {
                    "I have nothing further to add. ## Done".to_string()
                };
                // Gap #3 test seam: when the kernel negotiated streaming
                // (stream_deltas in params) and the fixture is armed, emit
                // the completion as ordered delta frames first.
                if deltas && params["stream_deltas"].as_bool() == Some(true) {
                    let bytes = completion.as_bytes();
                    let third = bytes.len().div_ceil(3).max(1);
                    for chunk in bytes.chunks(third) {
                        emit(serde_json::json!({
                            "delta": String::from_utf8_lossy(chunk).into_owned()
                        }));
                    }
                }
                // TUI proof seam: pace responses so live captures can
                // catch the rail mid-phase. Off by default.
                if let Ok(ms) = std::env::var("HS_SEQMODEL_DELAY_MS") {
                    if let Ok(ms) = ms.parse::<u64>() {
                        std::thread::sleep(std::time::Duration::from_millis(ms));
                    }
                }
                serde_json::json!({
                    "completion": completion,
                    "cached_tokens": 42,
                    "input_tokens": prompt.len() / 4 + 1,
                    "output_tokens": 9,
                    "cost_usd_micros": 700
                })
            }
            _ => serde_json::json!({"$error": "unknown method"}),
        },
    )
}
