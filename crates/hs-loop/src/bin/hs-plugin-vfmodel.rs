//! Test model "vfmodel": mission steps come from `HS_VF_SCRIPT` (JSONL, one
//! JSON tool call per line, cycling on the last); ADVERSARIAL VERIFIER calls
//! get a MECHANICAL verdict from the prompt's own sections - the fixture
//! that proves the verifier discriminates honest from dishonest work
//! without a canned answer.
//!
//! Judge rules (mechanical proxies for the prompt's honesty rules):
//! - no recorded test run in the LEDGER section -> refuted (gap)
//! - OBJECTIVE names task-N and ANSWER lacks TOKEN-N-SECRET -> refuted (bug)
//! - otherwise -> not refuted
include!("shared/sdk.rs");

fn main() {
    let script: Vec<String> =
        std::fs::read_to_string(std::env::var("HS_VF_SCRIPT").expect("HS_VF_SCRIPT"))
            .expect("script readable")
            .lines()
            .filter(|l| !l.trim().is_empty())
            .map(std::string::ToString::to_string)
            .collect();
    let n = std::cell::Cell::new(0usize);
    serve(
        "vfmodel",
        "model",
        &mut move |method, params| match method {
            "model.call" => {
                let __pv;
                let prompt = if let Some(p) = params["prompt"].as_str() { p } else {
                    __pv = hs_loop::msgfmt::prompt_view(&params);
                    __pv.as_str()
                };
                if prompt.contains("ADVERSARIAL VERIFIER") {
                    let ledger = prompt
                        .split("LEDGER (recorded evidence):")
                        .nth(1)
                        .and_then(|r| r.split("\nPRIOR_GAPS:").next())
                        .unwrap_or("");
                    let verified = ledger.contains("tests: \"");
                    let answer = prompt
                        .split("ANSWER:")
                        .nth(1)
                        .and_then(|r| r.split("\nLEDGER (recorded evidence):").next())
                        .unwrap_or("");
                    let token = prompt
                        .split("OBJECTIVE: ")
                        .nth(1)
                        .and_then(|r| r.lines().next())
                        .and_then(|o| {
                            o.trim()
                                .strip_prefix("task-")
                                .and_then(|n| n.parse::<usize>().ok())
                                .map(|n| format!("TOKEN-{n}-SECRET"))
                        });
                    let verdict = if !verified {
                        serde_json::json!({"refuted": true, "findings": [{"kind": "gap", "location": "ledger", "detail": "no recorded verification run - a claim without test evidence is fabricated"}], "blocking": "none"})
                    } else if let Some(t) = token {
                        if answer.contains(&t) {
                            serde_json::json!({"refuted": false, "findings": [], "blocking": "none"})
                        } else {
                            serde_json::json!({"refuted": true, "findings": [{"kind": "bug", "location": "answer", "detail": format!("answer does not deliver the objective ({t})")}], "blocking": "none"})
                        }
                    } else {
                        serde_json::json!({"refuted": false, "findings": [], "blocking": "none"})
                    };
                    return serde_json::json!({
                        "completion": serde_json::json!({"tool":"verdict.submit","args":verdict}).to_string(),
                        "input_tokens": prompt.len() / 4 + 1,
                        "output_tokens": 24,
                        "cost_usd_micros": 900
                    });
                }
                let i = n.get().min(script.len().saturating_sub(1));
                n.set(n.get() + 1);
                serde_json::json!({
                    "completion": script.get(i).cloned().unwrap_or_default(),
                    "input_tokens": prompt.len() / 4 + 1,
                    "output_tokens": 12,
                    "cost_usd_micros": 900
                })
            }
            _ => serde_json::json!({"$error": "unknown method"}),
        },
    );
}
