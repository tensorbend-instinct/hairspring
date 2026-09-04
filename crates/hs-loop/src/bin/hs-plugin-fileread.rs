//! SWE mission tool "repo.read": sandboxed read-only file access.
//! Workspace from HS_SWE_WORKSPACE (spawned per-mission by the runner).
include!("shared/sdk.rs");
fn main() {
    serve("repo.read", "tool", &mut |method, params| match method {
        "tool.call" => {
            let rel = params["args"]["path"].as_str().unwrap_or("");
            match std::env::var("HS_SWE_WORKSPACE") {
                Ok(ws) => match hs_loop::repotools::read_repo_file(
                    std::path::Path::new(&ws),
                    rel,
                ) {
                    Ok(v) => v,
                    Err(e) => serde_json::json!({"$error": e}),
                },
                Err(_) => serde_json::json!({"$error": "HS_SWE_WORKSPACE not set"}),
            }
        }
        _ => serde_json::json!({"$error": "unknown method"}),
    })
}
