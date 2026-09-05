//! SWE mission tool "edit.apply": incremental edits to a persistent candidate
//! worktree; returns the cumulative diff vs base on every call.
//! args: {diff} apply | {op:"diff"} read cumulative | {op:"reset"} discard.
//! Env: HS_SWE_WORKSPACE (required).
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
                    let diff = params["args"]["diff"].as_str().unwrap_or("");
                    if diff.is_empty() {
                        return serde_json::json!({"$error": "pass args.diff (unified diff) or args.op"});
                    }
                    hs_loop::editapply::apply(&ws, diff)
                }
            }
        }
        _ => serde_json::json!({"$error": "unknown method"}),
    })
}
