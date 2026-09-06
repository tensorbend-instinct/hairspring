//! SWE mission tool "repo.read": sandboxed read-only file access.
//! Workspace from HS_SWE_WORKSPACE (spawned per-mission by the runner).
include!("shared/sdk.rs");
fn main() {
    serve("repo.read", "tool", &mut |method, params| match method {
        "tool.call" => {
            let rel = params["args"]["path"].as_str().unwrap_or("");
            let start_line = params["args"]["start_line"].as_u64();
            let max_lines = params["args"]["max_lines"].as_u64();
            match std::env::var("HS_SWE_WORKSPACE") {
                Ok(ws) => {
                    let anchor_mode = std::env::var("HS_SWE_READ_ANCHORS").as_deref() == Ok("1");
                    let r = if anchor_mode {
                        hs_loop::editapply::anchored_read_window(
                            std::path::Path::new(&ws),
                            rel,
                            start_line,
                            max_lines,
                        )
                    } else {
                        hs_loop::repotools::read_repo_window(
                            std::path::Path::new(&ws),
                            rel,
                            start_line,
                            max_lines,
                        )
                    };
                    match r {
                        Ok(v) => v,
                        Err(e) => serde_json::json!({"$error": e}),
                    }
                }
                Err(_) => serde_json::json!({"$error": "HS_SWE_WORKSPACE not set"}),
            }
        }
        _ => serde_json::json!({"$error": "unknown method"}),
    })
}
