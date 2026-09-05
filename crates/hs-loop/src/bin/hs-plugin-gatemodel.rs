//! Evolution-bench fixture model "gatemodel": writes TOKEN-0-SECRET only
//! when the mission prompt carries the EVO-PREFLIGHT-LAW marker line, so a
//! prompt candidate's effect on mission outcome is measurable (the seam
//! recmodel established for compaction prompts).
include!("shared/sdk.rs");
fn main() {
    serve("gatemodel", "model", &mut |method, params| match method {
        "model.call" => {
            let prompt = params["prompt"].as_str().unwrap_or("");
            let path = prompt
                .lines()
                .find_map(|l| l.strip_prefix("ANSWER_PATH: "))
                .unwrap_or("")
                .to_string();
            let content = if prompt.contains("EVO-PREFLIGHT-LAW") {
                "TOKEN-0-SECRET"
            } else {
                "blind-wrong"
            };
            let completion = serde_json::json!({"tool":"answer.write","args":{"path":path,"content":content}});
            serde_json::json!({
                "completion": completion.to_string(),
                "input_tokens": prompt.len() / 4 + 1,
                "output_tokens": 12,
                "cost_usd_micros": 900
            })
        }
        _ => serde_json::json!({"$error": "unknown method"}),
    })
}
