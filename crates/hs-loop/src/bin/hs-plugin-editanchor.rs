//! SWE mission tool "edit.anchor": hashline anchor edits (Grok Build flavor)
//! to the persistent candidate worktree; validated against the current file,
//! returns a fresh-anchored snippet + cumulative diff. args: {path, edits}
//! apply | {op:"diff"} read | {op:"reset"} discard.
//! Env: HS_SWE_WORKSPACE (required).
include!("shared/sdk.rs");
fn main() {
    serve("edit.anchor", "tool", &mut |method, params| match method {
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
                    let path = params["args"]["path"].as_str().unwrap_or("");
                    if path.is_empty() {
                        return serde_json::json!({"$error": "pass path: the repo-relative file to edit"});
                    }
                    let edits = params["args"]["edits"].clone();
                    if edits.is_null() {
                        return serde_json::json!({"$error": "pass edits: [{op: replace|insert_after|write, anchor, content}, ...]"});
                    }
                    hs_loop::editapply::apply_anchor_edits(&ws, path, edits)
                }
            }
        }
        _ => serde_json::json!({"$error": "unknown method"}),
    });
}
