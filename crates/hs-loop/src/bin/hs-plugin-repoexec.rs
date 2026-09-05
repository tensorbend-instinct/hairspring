//! SWE mission tool "repo.exec": run an allowlisted lint/test command against
//! the current answer patch, applied to a scratch worktree (live workspace
//! untouched). See hs_loop::repexec for the contract.
//! Env: HS_SWE_WORKSPACE (required), HS_SWE_ANSWER (default answer path),
//!      HS_SWE_EXEC_TIMEOUT_SECS (default 120). Open shell - the bwrap
//!      sandbox is the only guard (Eric: zero list, isolation-only safety).
include!("shared/sdk.rs");
fn main() {
    serve("repo.exec", "tool", &mut |method, params| match method {
        "tool.call" => {
            let cmd = params["args"]["command"].as_str().unwrap_or("");
            let ws = match std::env::var("HS_SWE_WORKSPACE") {
                Ok(w) => std::path::PathBuf::from(w),
                Err(_) => return serde_json::json!({"$error": "HS_SWE_WORKSPACE not set"}),
            };
            let timeout: u64 = std::env::var("HS_SWE_EXEC_TIMEOUT_SECS")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(120);
            // inline diff wins (T4): the candidate under test, never a stale file
            if let Some(diff) = params["args"]["diff"].as_str() {
                return hs_loop::repexec::run_sandboxed_with_diff(&ws, diff, cmd, timeout);
            }
            let ans = params["args"]["path"]
                .as_str()
                .map(String::from)
                .or_else(|| std::env::var("HS_SWE_ANSWER").ok())
                .unwrap_or_default();
            if ans.is_empty() {
                return serde_json::json!({"$error": "no answer path: pass args.diff (inline unified diff), args.path (the ANSWER_PATH), or set HS_SWE_ANSWER"});
            }
            hs_loop::repexec::run_sandboxed(&ws, std::path::Path::new(&ans), cmd, timeout)
        }
        _ => serde_json::json!({"$error": "unknown method"}),
    })
}
