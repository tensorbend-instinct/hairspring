//! Scripted SWE-mission model for the offline integration proof. Attempt 1
//! submits with an untouched candidate (steering error - the 8609 empty-fence
//! failure class, exercised as a mission). Attempt 2 makes the fix with
//! edit.patch (Codex grammar). Attempt 3 submits; if the untested-answer gate
//! rejects (repo.exec present), attempt 4 verifies via repo.exec and attempt
//! 5 submits. Isolates the harness from model quality.
include!("shared/sdk.rs");

fn main() {
    serve("swemodel", "model", &mut |method, params| match method {
        "model.call" => {
            let __pv;
            let prompt = if let Some(p) = params["prompt"].as_str() { p } else {
                __pv = hs_loop::msgfmt::prompt_view(&params);
                __pv.as_str()
            };
            let path = prompt
                .lines()
                .find_map(|l| l.strip_prefix("ANSWER_PATH: "))
                .unwrap_or("")
                .to_string();
            // header format (fix 4): "ATTEMPT: step N of MAX, ..." - extract N
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
            let codex_gold =
                "*** Begin Patch\n*** Update File: code.txt\n@@\n-broken\n+fixed\n*** End Patch\n";
            // Blind-mode flow (prompt teaches .hs/checks): same mission, but
            // the agent declares its own checks before submitting - via
            // edit.patch, the only persistent write channel (repo.exec is a
            // per-call sandbox copy).
            let checks_patch =
                "*** Begin Patch\n*** Add File: .hs/checks\n+sh check.sh\n*** End Patch\n";
            // Terminal-bench flow (prompt teaches term.exec): the agent works
            // DIRECTLY on the live workdir (no candidate, no patch channel).
            // Attempt 1 submits with an empty summary (steering error). Attempt
            // 2 makes the fix via term.exec (skipped under HS_SWEMODEL_NOFIX
            // for the failing-checks gate). Attempt 3 declares .hs/checks and
            // runs them. Attempt 4 submits with a summary.
            let nofix = std::env::var("HS_SWEMODEL_NOFIX").is_ok();
            let completion = if prompt.contains("term.exec") {
                match attempt {
                    1 => serde_json::json!({"tool":"answer.submit","args":{"path":path,"summary":""}}),
                    2 => {
                        if nofix {
                            serde_json::json!({"tool":"term.exec","args":{"command":"true"}})
                        } else {
                            serde_json::json!({"tool":"term.exec","args":{"command":"printf 'fixed\n' > code.txt"}})
                        }
                    }
                    3 => serde_json::json!({"tool":"term.exec","args":{"command":"mkdir -p .hs && printf 'sh check.sh\n' > .hs/checks && sh .hs/checks"}}),
                    _ => serde_json::json!({"tool":"answer.submit","args":{"path":path,"summary":"fixed code.txt; check.sh green"}}),
                }
            } else if prompt.contains(".hs/checks") {
                match attempt {
                    2 => serde_json::json!({"tool":"edit.patch","args":{"patch":codex_gold}}),
                    3 => serde_json::json!({"tool":"edit.patch","args":{"patch":checks_patch}}),
                    4 => {
                        serde_json::json!({"tool":"repo.exec","args":{"command":"sh check.sh","diff":gold}})
                    }
                    // attempts 1 and anything past 4 submit
                    _ => serde_json::json!({"tool":"answer.submit","args":{"path":path}}),
                }
            } else {
                match attempt {
                    2 => serde_json::json!({"tool":"edit.patch","args":{"patch":codex_gold}}),
                    4 => {
                        serde_json::json!({"tool":"repo.exec","args":{"command":"sh check.sh","diff":gold}})
                    }
                    // attempts 1 and 3 (and anything past 4) submit
                    _ => serde_json::json!({"tool":"answer.submit","args":{"path":path}}),
                }
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
