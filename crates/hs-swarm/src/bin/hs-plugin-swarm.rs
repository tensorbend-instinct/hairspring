//! Tool plugin "agent.spawn" (Eric's five #5): delegates a mission to
//! a child sub-agent on the SAME substrate - the child runs the same
//! harness (kernel config, models, tools) on its own stream, linked
//! to the parent by a Spawn event. Blocks until the child finishes
//! and returns its report.
//!
//! args: {mission, parent_stream} - parent_stream is injected by the
//! loop at dispatch (the model never fabricates provenance).
//! env: HS_SWARM_LOG_ROOT + HS_SWARM_CONFIG (required; the REPL sets
//! both from its own opts), HS_SWARM_FEEDBACK ("1"), HS_SWARM_MAX_STEPS.
//! The plugin creates the child stream only; the parent's loop books
//! the Spawn event on its own stream (single-writer rule).
include!("../../../hs-loop/src/bin/shared/sdk.rs");

fn main() {
    serve("agent.spawn", "tool", &mut |method, params| match method {
        "tool.call" => {
            let mission = params["args"]["mission"].as_str().unwrap_or("");
            if mission.trim().is_empty() {
                return serde_json::json!({"$error": "pass mission: the delegated task"});
            }
            let parent = params["args"]["parent_stream"].as_str().unwrap_or("");
            let parent_id = match uuid::Uuid::parse_str(parent) {
                Ok(u) => u,
                Err(_) => {
                    return serde_json::json!({"$error": "parent_stream missing or invalid (the loop injects it)"});
                }
            };
            let log_root = match std::env::var("HS_SWARM_LOG_ROOT") {
                Ok(v) => v,
                Err(_) => return serde_json::json!({"$error": "HS_SWARM_LOG_ROOT not set"}),
            };
            let config = match std::env::var("HS_SWARM_CONFIG") {
                Ok(v) => v,
                Err(_) => return serde_json::json!({"$error": "HS_SWARM_CONFIG not set"}),
            };
            // Depth guard (fork-bomb lesson, 2026-09-08): a child that
            // tries to delegate gets a clean refusal instead of an
            // unbounded delegation tree. Depth rides the process env -
            // the child kernel's plugins inherit it from THIS process.
            let depth: u32 = std::env::var("HS_SWARM_DEPTH")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(0);
            let max_depth: u32 = std::env::var("HS_SWARM_MAX_DEPTH")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(1);
            if depth >= max_depth {
                return serde_json::json!({"$error": format!(
                    "delegation depth limit (max {max_depth}): run the subtask yourself with your own tools"
                )});
            }
            std::env::set_var("HS_SWARM_DEPTH", (depth + 1).to_string());
            let feedback = std::env::var("HS_SWARM_FEEDBACK").as_deref() == Ok("1");
            let max_steps: u32 = std::env::var("HS_SWARM_MAX_STEPS")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(16);
            let spawner = hs_swarm::Spawner::new(
                std::path::Path::new(&log_root),
                std::path::Path::new(&config),
                feedback,
                max_steps,
            );
            match spawner.spawn_child(parent_id, mission) {
                Ok((child, overhead_ms)) => match spawner.run_to_completion(&child) {
                    Ok(rep) => serde_json::json!({
                        "child_stream_id": rep.stream_id.to_string(),
                        "mission": rep.mission,
                        "passed": rep.passed,
                        "steps": rep.steps,
                        "cost_usd_micros": rep.cost_usd_micros,
                        "delegation_overhead_ms": overhead_ms,
                    }),
                    Err(e) => serde_json::json!({"$error": format!("child run: {e:?}")}),
                },
                Err(e) => serde_json::json!({"$error": format!("spawn: {e:?}")}),
            }
        }
        _ => serde_json::json!({"$error": "unknown method"}),
    })
}
