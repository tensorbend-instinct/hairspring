//! Gate-3 bench model "benchmodel": scripted, deterministic. It repairs
//! only from injected feedback ("expected token X"); blind it cycles a
//! fixed candidate list that never contains a repairable task's token.
//! This isolates the harness mechanism under test from model quality.
include!("shared/sdk.rs");

const BLIND: [&str; 6] = ["alpha", "bravo", "charlie", "delta", "echo", "foxtrot"];

fn main() {
    serve("benchmodel", "model", &mut |method, params| match method {
        "model.call" => {
            let prompt = params["prompt"].as_str().unwrap_or("");
            let path = prompt
                .lines()
                .find_map(|l| l.strip_prefix("ANSWER_PATH: "))
                .unwrap_or("")
                .to_string();
            // header format (fix 4): "ATTEMPT: step N of MAX, T-minus Xs,
            // $Y of $Z spent" - extract N; also accepts bare "ATTEMPT: N"
            let attempt: usize = prompt
                .lines()
                .find_map(|l| l.strip_prefix("ATTEMPT: "))
                .and_then(|v| {
                    let v = v.strip_prefix("step ").unwrap_or(v);
                    v.split(|c: char| !c.is_ascii_digit())
                        .next()
                        .and_then(|d| d.parse().ok())
                })
                .unwrap_or(1);
            let feedback_fix = prompt
                .lines()
                .find_map(|l| {
                    l.trim_start_matches("- ")
                        .strip_prefix("line 1: expected token ")
                })
                .map(|s| s.to_string());
            let content =
                feedback_fix.unwrap_or_else(|| BLIND[(attempt - 1) % BLIND.len()].to_string());
            let completion = serde_json::json!({
                "tool": "answer.write",
                "args": {"path": path, "content": content}
            });
            serde_json::json!({
                "completion": completion.to_string(),
                "input_tokens": prompt.len() / 4 + 1,
                "output_tokens": 12,
                "cost_usd_micros": 900
            })
        }
        _ => serde_json::json!({"$error": "unknown method"}),
    });
}
