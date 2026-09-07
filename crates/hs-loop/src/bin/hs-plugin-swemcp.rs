//! Test model "swemcp": drives an MCP-discovered tool through the real loop.
//! Attempt 1: mcp.fixture.echo {"text":"hello-via-mcp"}.
//! Attempt 2: repo.exec verifies the gold patch. Attempt 3: edit.patch.
//! Attempt 4: answer.submit (checker passes, mission ends).
include!("shared/sdk.rs");
fn main() {
    serve("swemcp", "model", &mut |method, params| match method {
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
            let completion = match attempt {
                1 => serde_json::json!({"tool":"mcp.fixture.echo","args":{"text":"hello-via-mcp"}}),
                2 => {
                    serde_json::json!({"tool":"repo.exec","args":{"command":"sh check.sh","diff":gold}})
                }
                3 => serde_json::json!({"tool":"edit.patch","args":{"patch":
                    "*** Begin Patch\n*** Update File: code.txt\n@@\n-broken\n+fixed\n*** End Patch\n"}}),
                _ => serde_json::json!({"tool":"answer.submit","args":{"path":path}}),
            };
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
