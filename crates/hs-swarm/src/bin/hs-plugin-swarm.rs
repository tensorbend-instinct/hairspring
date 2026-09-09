//! Tool plugin "agent.spawn" + "agent.spawn_poll" (async delegation,
//! Eric ruling 2026-09-08): agent.spawn starts a child sub-agent on
//! the SAME substrate (kernel config, models, tools) on its own
//! stream and returns IMMEDIATELY (status running); the child drives
//! to completion on a dedicated thread in this process. The loop
//! learns the outcome by polling agent.spawn_poll at step boundaries.
//!
//! args (spawn): {mission, parent_stream, child_stream_id, model?} -
//! parent_stream + child_stream_id are injected by the loop at
//! dispatch; the loop books Spawn BEFORE calling (atomic provenance),
//! so the child stream is created with exactly the loop-minted id.
//! args (poll): {child_stream_id} -> running | done + report.
//!
//! Registry ON DISK (<log_root>/swarm/): spawn.json at start,
//! report.json at completion. The kernel may run spawn and poll in
//! separate plugin processes - disk is the only honest shared state,
//! and it matches the crash-atomicity contract: a missing report with
//! a present spawn marker is a child still running (or a dead plugin
//! process, which fails the parent's calls loudly anyway).
//!
//! env: HS_SWARM_LOG_ROOT + HS_SWARM_CONFIG (required; the REPL sets
//! both from its own opts), HS_SWARM_FEEDBACK ("1"), HS_SWARM_MAX_STEPS,
//! HS_SWARM_DEPTH / HS_SWARM_MAX_DEPTH (fork-bomb guard, 2026-09-08),
//! HS_SWARM_MAX_CHILDREN (concurrency cap, default 4).
include!("../../../hs-loop/src/bin/shared/sdk.rs");

fn swarm_dir(log_root: &str) -> std::path::PathBuf {
    std::path::Path::new(log_root).join("swarm")
}

fn running_count(dir: &std::path::Path) -> usize {
    std::fs::read_dir(dir)
        .map(|rd| {
            rd.filter_map(|e| e.ok())
                .filter(|e| {
                    let n = e.file_name().to_string_lossy().to_string();
                    n.ends_with(".spawn.json")
                        && !dir.join(n.replace(".spawn.json", ".report.json")).exists()
                })
                .count()
        })
        .unwrap_or(0)
}

fn main() {
    // One binary, two registered names: the kernel validates the
    // describe handshake against the config slot name, so the poll
    // entry spawns us as `hs-plugin-swarm --as agent.spawn_poll`.
    // Both slots share state through the on-disk registry only.
    let argv: Vec<String> = std::env::args().collect();
    let name: &'static str = match argv.iter().position(|a| a == "--as") {
        Some(i) if argv.get(i + 1).map(|s| s.as_str()) == Some("agent.spawn_poll") => {
            "agent.spawn_poll"
        }
        _ => "agent.spawn",
    };
    serve(name, "tool", &mut |method, params| match method {
        "tool.call" => {
            let args = &params["args"];
            let mission = args["mission"].as_str().unwrap_or("");
            let log_root = match std::env::var("HS_SWARM_LOG_ROOT") {
                Ok(v) => v,
                Err(_) => return serde_json::json!({"$error": "HS_SWARM_LOG_ROOT not set"}),
            };
            // Poll: report a delegated child's state from the disk
            // registry. Never booked by the kernel's quiet query path.
            if args["child_stream_id"].is_string() && mission.is_empty() {
                let cid = args["child_stream_id"].as_str().unwrap_or("");
                let dir = swarm_dir(&log_root);
                if let Ok(s) = std::fs::read_to_string(dir.join(format!("{cid}.report.json"))) {
                    let mut v: serde_json::Value =
                        serde_json::from_str(&s).unwrap_or_else(|_| serde_json::json!({}));
                    v["status"] = serde_json::json!("done");
                    return v;
                }
                if dir.join(format!("{cid}.spawn.json")).exists() {
                    return serde_json::json!({"status": "running"});
                }
                return serde_json::json!({"$error": format!(
                    "unknown child_stream_id {cid}: no spawn marker under {}/swarm", log_root
                )});
            }
            if mission.trim().is_empty() {
                return serde_json::json!({"$error": "pass mission: the delegated task"});
            }
            let parent = args["parent_stream"].as_str().unwrap_or("");
            let parent_id = match uuid::Uuid::parse_str(parent) {
                Ok(u) => u,
                Err(_) => {
                    return serde_json::json!({"$error": "parent_stream missing or invalid (the loop injects it)"});
                }
            };
            let child = args["child_stream_id"].as_str().unwrap_or("");
            let child_id = match uuid::Uuid::parse_str(child) {
                Ok(u) => u,
                Err(_) => {
                    return serde_json::json!({"$error": "child_stream_id missing or invalid (the loop mints it)"});
                }
            };
            let model = args["model"].as_str().map(|m| m.to_string());
            let config = match std::env::var("HS_SWARM_CONFIG") {
                Ok(v) => v,
                Err(_) => return serde_json::json!({"$error": "HS_SWARM_CONFIG not set"}),
            };
            // Depth guard (fork-bomb lesson, 2026-09-08): a child that
            // tries to delegate gets a clean refusal instead of an
            // unbounded delegation tree. Depth travels IN THE ARGS,
            // injected by the loop (its own depth, seeded from
            // HS_SWARM_DEPTH at session start) - never env-mutated:
            // this process is long-lived and spawns many children.
            let depth: u32 = match args["depth"].as_u64() {
                Some(d) => d as u32,
                None => {
                    return serde_json::json!({"$error": "depth missing (the loop injects it)"});
                }
            };
            let max_depth: u32 = std::env::var("HS_SWARM_MAX_DEPTH")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(1);
            if depth >= max_depth {
                return serde_json::json!({"$error": format!(
                    "delegation depth limit (max {max_depth}): run the subtask yourself with your own tools"
                )});
            }
            // Concurrency cap (D4): bounded fan-out per parent.
            let max_children: usize = std::env::var("HS_SWARM_MAX_CHILDREN")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(4);
            let dir = swarm_dir(&log_root);
            let _ = std::fs::create_dir_all(&dir);
            if running_count(&dir) >= max_children {
                return serde_json::json!({"$error": format!(
                    "delegation concurrency limit (max {max_children} running children): wait for a delegation update, then retry"
                )});
            }
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
            match spawner.spawn_child(parent_id, child_id, mission, model.as_deref(), depth + 1) {
                Ok((child, overhead_ms)) => {
                    // Registry marker BEFORE the thread starts: a poll
                    // never sees a child the registry doesn't know.
                    let marker = serde_json::json!({
                        "child_stream_id": child_id.to_string(),
                        "parent_stream": parent_id.to_string(),
                        "mission": mission,
                        "model": model,
                        "started_at_ms": std::time::SystemTime::now()
                            .duration_since(std::time::UNIX_EPOCH)
                            .map(|d| d.as_millis() as i64)
                            .unwrap_or(0),
                    });
                    if let Err(e) = std::fs::write(
                        dir.join(format!("{child_id}.spawn.json")),
                        serde_json::to_string_pretty(&marker).unwrap_or_default(),
                    ) {
                        return serde_json::json!({"$error": format!("registry marker: {e}")});
                    }
                    let report_path = dir.join(format!("{child_id}.report.json"));
                    std::thread::spawn(move || {
                        let spawner = hs_swarm::Spawner::new(
                            std::path::Path::new(&log_root),
                            std::path::Path::new(&config),
                            feedback,
                            max_steps,
                        );
                        let report = std::panic::catch_unwind(
                            std::panic::AssertUnwindSafe(|| spawner.run_to_completion(&child)),
                        );
                        let v = match report {
                            Ok(Ok(rep)) => serde_json::json!({
                                "child_stream_id": rep.stream_id.to_string(),
                                "mission": rep.mission,
                                "model": rep.model,
                                "passed": rep.passed,
                                "steps": rep.steps,
                                "cost_usd_micros": rep.cost_usd_micros,
                                "delegation_overhead_ms": overhead_ms,
                            }),
                            Ok(Err(e)) => serde_json::json!({
                                "child_stream_id": child.stream_id.to_string(),
                                "mission": child.mission,
                                "passed": false,
                                "error": format!("child run: {e:?}"),
                            }),
                            Err(_) => serde_json::json!({
                                "child_stream_id": child.stream_id.to_string(),
                                "mission": child.mission,
                                "passed": false,
                                "error": "child panicked (contained by catch_unwind)",
                            }),
                        };
                        let _ = std::fs::write(
                            &report_path,
                            serde_json::to_string_pretty(&v).unwrap_or_default(),
                        );
                    });
                    serde_json::json!({
                        "child_stream_id": child_id.to_string(),
                        "status": "running",
                        "model": model,
                        "delegation_overhead_ms": overhead_ms,
                    })
                }
                Err(e) => serde_json::json!({"$error": format!("spawn: {e:?}")}),
            }
        }
        _ => serde_json::json!({"$error": "unknown method"}),
    })
}
