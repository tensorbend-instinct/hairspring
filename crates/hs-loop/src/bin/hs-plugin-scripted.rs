//! Phase 1 test model "seqmodel": scripted completions from a file
//! (HS_SEQMODEL_SCRIPT), one JSON line per model.call, cycling on the last
//! line when the script runs out. This isolates loop behavior (supervisor
//! aborts, progress booking) from any real model.
include!("shared/sdk.rs");

fn main() {
    let script: Vec<String> = std::fs::read_to_string(std::env::var("HS_SEQMODEL_SCRIPT").unwrap())
        .unwrap()
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| l.to_string())
        .collect();
    assert!(!script.is_empty(), "empty seqmodel script");
    let mut n = 0usize;
    serve("scripted", "model", &mut move |method, params| match method {
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
            let completion = script[n.min(script.len() - 1)].clone();
            n += 1;
            serde_json::json!({
                "completion": completion,
                "cached_tokens": 42,
                "input_tokens": prompt.len() / 4 + 1,
                "output_tokens": 9,
                "cost_usd_micros": 700
            })
        }
        _ => serde_json::json!({"$error": "unknown method"}),
    })
}
