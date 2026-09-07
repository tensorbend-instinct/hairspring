//! Blind-mode checker plugin "checker.run": the mission's stop authority is
//! the agent's OWN declared checks (.hs/checks in the candidate worktree) -
//! never ground-truth FAIL_TO_PASS (Eric 2026-09-07). The adversarial
//! verifier audits sufficiency from the transcript.
//! Env: HS_SWE_WORKSPACE (required).
include!("shared/sdk.rs");
fn main() {
    serve("checker.run", "tool", &mut |method, _| match method {
        "checker.run" | "tool.call" => match std::env::var("HS_SWE_WORKSPACE") {
            Ok(w) => hs_loop::selfcheck::check(std::path::Path::new(&w)),
            Err(_) => serde_json::json!({"$error": "HS_SWE_WORKSPACE not set"}),
        },
        _ => serde_json::json!({"$error": "unknown method"}),
    })
}
