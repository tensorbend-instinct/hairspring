//! Critic-mode checker plugin "checker.run" (Eric 2026-09-07): phase 1 runs
//! the agent's own declared checks (selfcheck direct mode); phase 2 hands
//! the submission to an INDEPENDENT critic context whose only job is to
//! refute it against the original instruction. Green requires both. Every
//! abnormal path fails closed.
//! Env: HS_SWE_WORKSPACE (required), HS_TB_INSTRUCTION_FILE (required),
//! HS_TB_ANSWER_FILE, HS_CRITIC_TRACE, HS_CRITIC_* caps, HS_DEEPSEEK_* keys.
include!("shared/sdk.rs");
fn main() {
    serve("checker.run", "tool", &mut |method, _| match method {
        "checker.run" | "tool.call" => match std::env::var("HS_SWE_WORKSPACE") {
            Ok(w) => hs_loop::critic::checker_gate(std::path::Path::new(&w)),
            Err(_) => serde_json::json!({"$error": "HS_SWE_WORKSPACE not set"}),
        },
        _ => serde_json::json!({"$error": "unknown method"}),
    })
}
