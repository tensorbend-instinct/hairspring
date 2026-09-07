//! Test model "recmodel": dumps every prompt it receives to the file in
//! env REC_DUMP, then follows a fixed script. REC_MODE=pressure: four
//! bigread.read calls, then writes the secret ONLY once a prompt carries a
//! COMPACTED transcript summary. REC_MODE=small: one probe.read, then
//! writes the secret when the marker flows back - the no-pressure control.
include!("shared/sdk.rs");
fn main() {
    serve("recmodel", "model", &mut |method, params| match method {
        "model.call" => {
            let __pv;
            let prompt = match params["prompt"].as_str() {
                Some(p) => p,
                None => {
                    __pv = hs_loop::msgfmt::prompt_view(&params);
                    __pv.as_str()
                }
            };
            if let Ok(dump) = std::env::var("REC_DUMP") {
                use std::io::Write;
                let mut f = std::fs::OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open(dump)
                    .unwrap();
                writeln!(f, "===PROMPT===\n{prompt}").unwrap();
            }
            let mode = std::env::var("REC_MODE").unwrap_or_else(|_| "pressure".into());
            let path = prompt
                .lines()
                .find_map(|l| l.strip_prefix("ANSWER_PATH: "))
                .unwrap_or("")
                .to_string();
            // header format (fix 4): "ATTEMPT: step N of MAX, T-minus Xs,
            // $Y of $Z spent" - extract N; also accepts the bare "ATTEMPT: N"
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
            let completion = if mode == "small" {
                if attempt == 1 {
                    serde_json::json!({"tool":"probe.read","args":{"path":"x"}})
                } else if prompt.contains("MARKER-777") {
                    serde_json::json!({"tool":"answer.write","args":{"path":path,"content":"TOKEN-0-SECRET"}})
                } else {
                    serde_json::json!({"tool":"answer.write","args":{"path":path,"content":"blind-wrong"}})
                }
            } else if (1..=4).contains(&attempt) {
                serde_json::json!({"tool":"bigread.read","args":{"page":attempt}})
            } else if prompt.contains("COMPACTED") {
                serde_json::json!({"tool":"answer.write","args":{"path":path,"content":"TOKEN-0-SECRET"}})
            } else {
                serde_json::json!({"tool":"answer.write","args":{"path":path,"content":"no-compaction-seen"}})
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
