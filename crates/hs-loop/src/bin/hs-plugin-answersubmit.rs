//! SWE mission tool "answer.submit": computes the candidate worktree's
//! cumulative diff with git and writes it to the answer path. The model
//! supplies NO content - the submission is exactly what it built and
//! verified, never hand-written diff text. args: {path}.
//! Env: HS_SWE_WORKSPACE (required).
include!("shared/sdk.rs");
fn main() {
    serve("answer.submit", "tool", &mut |method, params| match method {
        "tool.call" => {
            let ws = match std::env::var("HS_SWE_WORKSPACE") {
                Ok(w) => std::path::PathBuf::from(w),
                Err(_) => return serde_json::json!({"$error": "HS_SWE_WORKSPACE not set"}),
            };
            let path = params["args"]["path"].as_str().unwrap_or("");
            if path.is_empty() {
                return serde_json::json!({"$error": "pass path: the ANSWER_PATH value"});
            }
            hs_loop::editapply::answer_submit(&ws, std::path::Path::new(path))
        }
        _ => serde_json::json!({"$error": "unknown method"}),
    });
}
