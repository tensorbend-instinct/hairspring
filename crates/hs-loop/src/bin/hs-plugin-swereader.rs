//! Test model "swereader": step 1 reads the repo via repo.read, step 2
//! verifies the gold patch inline via repo.exec (hard answer gate), step 3
//! writes it. Exercises the driver-configured repotools through the kernel
//! in the driver E2E (seam gap C3).
include!("shared/sdk.rs");
fn main() {
    serve("swereader", "model", &mut |method, params| match method {
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
            let gold = std::env::var("HS_SWE_GOLD_PATCH_FILE")
                .ok()
                .and_then(|f| std::fs::read_to_string(f).ok())
                .unwrap_or_default();
            let completion = if attempt == 1 {
                serde_json::json!({"tool":"repo.read","args":{"path":"code.txt"}})
            } else if attempt == 2 {
                serde_json::json!({"tool":"repo.exec","args":{"command":"sh check.sh","diff":gold}})
            } else {
                serde_json::json!({"tool":"answer.write","args":{"path":path,"content":format!("```diff\n{gold}\n```")}})
            };
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
