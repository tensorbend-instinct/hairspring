//! SWE mission tool "edit.patch": Codex apply_patch-grammar edits to the
//! persistent candidate worktree; returns the cumulative diff vs base on
//! every call. args: {patch} apply | {op:"diff"} read | {op:"reset"} discard.
//! The model never writes unified-diff syntax (corrupt-patch failure class,
//! measured 2026-09-06). Env: HS_SWE_WORKSPACE (required).
include!("shared/sdk.rs");
fn main() {
    serve("edit.patch", "tool", &mut |method, params| match method {
        "tool.call" => {
            let ws = match std::env::var("HS_SWE_WORKSPACE") {
                Ok(w) => std::path::PathBuf::from(w),
                Err(_) => return serde_json::json!({"$error": "HS_SWE_WORKSPACE not set"}),
            };
            match params["args"]["op"].as_str() {
                Some("diff") => hs_loop::editapply::cumulative_diff(&ws),
                Some("reset") => hs_loop::editapply::reset(&ws),
                Some(other) => serde_json::json!({"$error": format!("unknown op '{other}'")}),
                None => match params["args"]["patch"].as_str() {
                    Some(p) => hs_loop::editapply::apply_codex_patch(&ws, p),
                    None => serde_json::json!({"$error": "pass patch: one apply_patch text (*** Begin Patch ... *** End Patch)"}),
                },
            }
        }
        _ => serde_json::json!({"$error": "unknown method"}),
    });
}
