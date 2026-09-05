//! Evolution-bench fixture model "gatemodel": writes TOKEN-0-SECRET only
//! when the mission prompt carries the EVO-PREFLIGHT-LAW marker line, so a
//! prompt candidate's effect on mission outcome is measurable (the seam
//! recmodel established for compaction prompts).
include!("shared/sdk.rs");
fn main() {
    serve("gatemodel", "model", &mut |method, params| match method {
        "model.call" => {
            let __pv;
            let prompt = match params["prompt"].as_str() {
                Some(p) => p,
                None => {
                    __pv = hs_loop::msgfmt::prompt_view(&params);
                    __pv.as_str()
                }
            };
            let path = prompt
                .lines()
                .find_map(|l| l.strip_prefix("ANSWER_PATH: "))
                .unwrap_or("")
                .to_string();
            let content = if prompt.contains("EVO-PREFLIGHT-LAW") {
                // the checker expects TOKEN-<n>-SECRET for task-<n>; the
                // task id rides in the answer path (.../work/task-N/answer.txt)
                let n = path.split("/work/task-").nth(1)
                    .and_then(|r| r.split('/').next())
                    .and_then(|s| s.parse::<usize>().ok())
                    .unwrap_or(0);
                format!("TOKEN-{n}-SECRET")
            } else {
                "blind-wrong".to_string()
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
