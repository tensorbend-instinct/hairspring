//! SWE mission tool "repo.exec": run lint/test commands against the current
//! answer patch (scratch worktree; live workspace untouched), or - with no
//! diff/path - general shell commands against a pristine scratch clone
//! (scratch-shell mode). See `hs_loop::repexec` for the contract.
//! Env: `HS_SWE_WORKSPACE` (required), `HS_SWE_ANSWER` (default answer path),
//!      `HS_SWE_EXEC_TIMEOUT_SECS` (default 120). Open shell - the bwrap
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
            // Scratch-shell mode (Eric 2026-09-05, post-verify17092): no diff
            // and no answer path = the model is exploring (git log, grep,
            // pwd), not testing a candidate. Run against a pristine clone
            // instead of erroring. Only the diff paths above count as
            // candidate verification for the answer gate.
            if ans.is_empty() {
                return hs_loop::repexec::run_sandboxed_no_patch(&ws, cmd, timeout);
            }
            hs_loop::repexec::run_sandboxed(&ws, std::path::Path::new(&ans), cmd, timeout)
        }
        _ => serde_json::json!({"$error": "unknown method"}),
    });
}
