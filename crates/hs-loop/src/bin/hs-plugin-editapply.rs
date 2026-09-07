//! SWE mission tool "edit.apply": search/replace edits to a persistent
//! candidate worktree; returns the cumulative diff vs base on every call.
//! args: {edits:[{path, old, new}]} apply | {op:"diff"} read cumulative |
//! {op:"reset"} discard. The raw unified-diff arg is retired (corrupt-patch
//! failure class, measured 2026-09-05). Env: HS_SWE_WORKSPACE (required).
include!("shared/sdk.rs");
fn main() {
    serve("edit.apply", "tool", &mut |method, params| match method {
        "tool.call" => {
            let ws = match std::env::var("HS_SWE_WORKSPACE") {
                Ok(w) => std::path::PathBuf::from(w),
                Err(_) => return serde_json::json!({"$error": "HS_SWE_WORKSPACE not set"}),
            };
            match params["args"]["op"].as_str() {
                Some("diff") => hs_loop::editapply::cumulative_diff(&ws),
                Some("reset") => hs_loop::editapply::reset(&ws),
                Some(other) => serde_json::json!({"$error": format!("unknown op '{other}'")}),
                None => {
                    if let Some(edits) = params["args"]["edits"].as_array() {
                        let mut blocks = Vec::new();
                        for (i, e) in edits.iter().enumerate() {
                            let p = e["path"].as_str();
                            let o = e["old"].as_str();
                            let n = e["new"].as_str();
                            match (p, o, n) {
                                (Some(p), Some(o), Some(n)) => {
                                    blocks.push(hs_loop::editapply::EditBlock {
                                        path: p.to_string(),
                                        old: o.to_string(),
                                        new: n.to_string(),
                                    })
                                }
                                _ => {
                                    return serde_json::json!({"$error": format!("edits[{i}] needs path, old and new strings")});
                                }
                            }
                        }
                        hs_loop::editapply::apply_blocks(&ws, &blocks)
                    } else if params["args"]["diff"].is_string() {
                        serde_json::json!({"$error": "edit.apply no longer takes a unified diff - pass edits: [{path, old, new}] search/replace blocks: old copied verbatim from repo.read, unique in the file, new the replacement. No line numbers, no diff syntax."})
                    } else {
                        serde_json::json!({"$error": "pass args.edits ([{path, old, new}]) or args.op"})
                    }
                }
            }
        }
        _ => serde_json::json!({"$error": "unknown method"}),
    })
}
