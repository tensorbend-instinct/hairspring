//! Scripted model plugin (serves "scripted"): completions replayed from a
//! file (`HS_SEQMODEL_SCRIPT`), one JSON line per model.call, cycling on
//! the last line when the script runs out. Two jobs: the suite's
//! deterministic model (loop mechanics isolated from any real model), and
//! the shipped OFFLINE trial model - the example config wires it so a new
//! user can run a full mission with zero network (see the README's offline
//! trial). The script loads lazily per call: without the env var the
//! plugin stays inert and names what to set.
include!("shared/sdk.rs");

/// The `ANSWER_PATH` value out of the operator prompt's volatile tail.
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

/// The script loads LAZILY, per call, never at startup (stranger-path
/// burns 2026-09-09, runs 1+2): the shipped example config wires this
/// plugin as the offline trial model, and a startup panic on the missing
/// env killed the whole kernel - EVERY run, key or no key - with a naked
/// `plugin exited (EOF)`. Inert until called: describe always answers; a
/// call without a script gets a `$error` naming exactly what to set.
fn load_script() -> Result<Vec<String>, String> {
    let path = std::env::var("HS_SEQMODEL_SCRIPT").map_err(|_| {
        "HS_SEQMODEL_SCRIPT not set - point it at a .jsonl file of scripted          model replies (the repo ships examples/seqmodel-demo.jsonl)"
            .to_string()
    })?;
    let text = std::fs::read_to_string(&path)
        .map_err(|e| format!("HS_SEQMODEL_SCRIPT {path}: {e}"))?;
    let script: Vec<String> = text
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(std::string::ToString::to_string)
        .collect();
    if script.is_empty() {
        return Err(format!("HS_SEQMODEL_SCRIPT {path}: empty script"));
    }
    Ok(script)
}

fn main() {
    register_preflight(|| match std::env::var("HS_SEQMODEL_SCRIPT") {
        Ok(v) if !v.trim().is_empty() => Ok(()),
        _ => Err(
            "offline model needs HS_SEQMODEL_SCRIPT=<script.jsonl> - see the README's offline quickstart"
                .to_string(),
        ),
    });
    let deltas = std::env::var("HS_SEQMODEL_DELTAS").as_deref() == Ok("1");
    let mut n = 0usize;
    // Per-instance identity: a fixture wiring the same binary as
    // several models (model-override tests) names each copy via env.
    let serve_name =
        std::env::var("HS_SCRIPTED_NAME").unwrap_or_else(|_| "scripted".to_string());
    // serve_ext wants 'static; the plugin process lives exactly as
    // long as this one leaked name.
    let serve_name: &'static str = Box::leak(serve_name.into_boxed_str());
    serve_ext(
        serve_name,
        "model",
        &mut move |method, params, emit| match method {
            "model.call" => {
                let __pv;
                let prompt = if let Some(p) = params["prompt"].as_str() { p } else {
                    __pv = hs_loop::msgfmt::prompt_view(&params);
                    __pv.as_str()
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
                let script = match load_script() {
                    Ok(s) => s,
                    Err(e) => return serde_json::json!({"$error": e}),
                };
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
                                r#"{{"tool":"answer.submit","args":{{"path":"{path}","summary":"scripted answer: mission complete"}}}}"#
                            ),
                            Some(t) => format!(
                                r#"{{"tool":"{t}","args":{{"path":"{path}","content":"scripted answer: mission complete"}}}}"#
                            ),
                            None => "I have no answer tool to submit with. ## Done".to_string(),
                        }
                    }
                } else {
                    "I have nothing further to add. ## Done".to_string()
                };
                // Reasoning seam (Eric 2026-09-10: the surface shows the
                // provider's REAL reasoning, never fabricated): a script
                // line of {"completion": ..., "reasoning": ...} supplies
                // the scripted provider's reasoning_content. Plain lines
                // keep their exact replay contract.
                let (completion, reasoning) =
                    match serde_json::from_str::<serde_json::Value>(&completion) {
                        Ok(v) if v.get("completion").and_then(|c| c.as_str()).is_some() => (
                            v["completion"].as_str().unwrap_or_default().to_string(),
                            v["reasoning"].as_str().unwrap_or("").to_string(),
                        ),
                        _ => (completion, String::new()),
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
                if let Ok(ms) = std::env::var("HS_SEQMODEL_DELAY_MS")
                    && let Ok(ms) = ms.parse::<u64>() {
                        std::thread::sleep(std::time::Duration::from_millis(ms));
                    }
                serde_json::json!({
                    "completion": completion,
                    "reasoning_content": reasoning,
                    "cached_tokens": 42,
                    "input_tokens": prompt.len() / 4 + 1,
                    "output_tokens": 9,
                    "cost_usd_micros": 700
                })
            }
            _ => serde_json::json!({"$error": "unknown method"}),
        },
    );
}
