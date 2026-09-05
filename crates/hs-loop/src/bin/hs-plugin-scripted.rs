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
            let prompt = params["prompt"].as_str().unwrap_or("");
            let completion = script[n.min(script.len() - 1)].clone();
            n += 1;
            serde_json::json!({
                "completion": completion,
                "input_tokens": prompt.len() / 4 + 1,
                "output_tokens": 9,
                "cost_usd_micros": 700
            })
        }
        _ => serde_json::json!({"$error": "unknown method"}),
    })
}
