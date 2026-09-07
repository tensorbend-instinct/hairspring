//! SWE mission tool "repo.search": substring search with file:line hits,
//! workspace-sandboxed, .git/target skipped, 100-hit cap.
include!("shared/sdk.rs");
fn main() {
    serve("repo.search", "tool", &mut |method, params| match method {
        "tool.call" => {
            let pat = params["args"]["pattern"].as_str().unwrap_or("");
            match std::env::var("HS_SWE_WORKSPACE") {
                Ok(ws) => match hs_loop::repotools::search_repo(std::path::Path::new(&ws), pat) {
                    Ok(v) => v,
                    Err(e) => serde_json::json!({"$error": e}),
                },
                Err(_) => serde_json::json!({"$error": "HS_SWE_WORKSPACE not set"}),
            }
        }
        _ => serde_json::json!({"$error": "unknown method"}),
    })
}
