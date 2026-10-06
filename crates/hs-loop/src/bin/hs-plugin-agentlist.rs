//! Tool "agent.list": see hs_loop::agentctl.
include!("shared/sdk.rs");
fn main() {
    serve("agent.list", "tool", &mut |method, params| match method {
        "tool.call" => {
            let args = &params["args"];
            let Ok(root) = std::env::var("HS_SWARM_LOG_ROOT") else {
                return serde_json::json!({"$error": "HS_SWARM_LOG_ROOT not set"});
            };
            hs_loop::agentctl::list(std::path::Path::new(&root))
        }
        _ => serde_json::json!({"$error": "unknown method"}),
    });
}
