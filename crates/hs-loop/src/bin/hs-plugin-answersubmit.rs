//! SWE mission tool "answer.submit": computes the candidate worktree's
//! cumulative diff with git and writes it to the answer path. The model
//! supplies NO content - the submission is exactly what it built and
//! verified, never hand-written diff text. args: {path}.
//! Env: `HS_SWE_WORKSPACE` (required).
include!("shared/sdk.rs");
fn main() {
    serve(
        "answer.submit",
        "tool",
        &mut |method, params| match method {
            "tool.call" => {
                // Terminal-bench mode (HS_ANSWER_RAW=1): the deliverable is
                // the live container state, not a patch - the submission is a
                // human-readable completion summary. Still no hand-written
                // diffs: there is no diff channel in tb mode at all.
                if std::env::var("HS_ANSWER_RAW").as_deref() == Ok("1") {
                    let path = params["args"]["path"].as_str().unwrap_or("");
                    let summary = params["args"]["summary"].as_str().unwrap_or("");
                    if path.is_empty() {
                        return serde_json::json!({"$error": "pass path: the ANSWER_PATH value"});
                    }
                    if summary.trim().is_empty() {
                        return serde_json::json!({"$error": "pass summary: what you changed and how you verified it"});
                    }
                    let path = match hs_loop::projectroot::confine_write(
                        std::path::Path::new(path),
                        "answer.submit",
                    ) {
                        Ok(p) => p,
                        Err(e) => return e,
                    };
                    return match std::fs::write(&path, summary) {
                        Ok(()) => serde_json::json!({"written": true, "bytes": summary.len()}),
                        Err(e) => serde_json::json!({"$error": format!("write: {e}")}),
                    };
                }
                let ws = match std::env::var("HS_SWE_WORKSPACE") {
                    Ok(w) => std::path::PathBuf::from(w),
                    Err(_) => return serde_json::json!({"$error": "HS_SWE_WORKSPACE not set"}),
                };
                let path = params["args"]["path"].as_str().unwrap_or("");
                if path.is_empty() {
                    return serde_json::json!({"$error": "pass path: the ANSWER_PATH value"});
                }
                let path = match hs_loop::projectroot::confine_write(
                    std::path::Path::new(path),
                    "answer.submit",
                ) {
                    Ok(p) => p,
                    Err(e) => return e,
                };
                hs_loop::editapply::answer_submit(&ws, &path)
            }
            _ => serde_json::json!({"$error": "unknown method"}),
        },
    );
}
