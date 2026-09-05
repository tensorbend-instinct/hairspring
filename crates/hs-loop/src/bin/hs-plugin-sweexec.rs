//! Test model "sweexec": drives repo.exec through the real loop, post-gate.
//! Attempt 1: repo.exec a deliberately corrupt INLINE patch (bad framing)
//!   - the free apply-error feedback path, booked on the audit stream.
//! Attempt 2: repo.exec the gold INLINE patch - the honest pre-submit
//!   verification the hard answer.write gate requires.
//! Attempt 3: answer.write the gold patch (accepted, checker passes).
include!("shared/sdk.rs");
fn main() {
    serve("sweexec", "model", &mut |method, params| match method {
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
            let gold = std::env::var("HS_SWE_GOLD_PATCH_FILE")
                .ok()
                .and_then(|f| std::fs::read_to_string(f).ok())
                .unwrap_or_default();
            let completion = match attempt {
                1 => serde_json::json!({"tool":"repo.exec","args":{"command":"sh check.sh",
                    "diff":"--- a/code.txt\n+++ b/code.txt\n@@ -1 +1 @@\n-WRONGCONTEXT\n+fixed\n"}}),
                2 => serde_json::json!({"tool":"repo.exec","args":{"command":"sh check.sh",
                    "diff":gold}}),
                _ => serde_json::json!({"tool":"answer.write","args":{"path":path,
                    "content":format!("```diff\n{gold}\n```")}}),
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
