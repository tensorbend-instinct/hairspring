//! Async delegation (Eric ruling 2026-09-08: "async delegation, proper
//! delegation event paradigm, atomic safe, best practices"). The #5
//! shape was SYNCHRONOUS: agent.spawn blocked the parent until the
//! child finished, so SubAgentSpawned/Finished fired back-to-back
//! post-hoc and the panel could never show a child mid-run.
//!
//! Contract (D1-D5, approved by Eric via Main):
//! - agent.spawn returns IMMEDIATELY (status running); children run
//!   CONCURRENTLY on plugin-side threads.
//! - The loop mints child_stream_id and books Spawn BEFORE the call
//!   (atomic: a crash anywhere leaves consistent provenance).
//! - Outcomes arrive via agent.spawn_poll at step boundaries:
//!   SubAgentFinished + cost fold + an ungated delegation update.
//! - A passing close JOINS running children (books stay honest).
//! - Depth guard + a concurrency cap bound the tree.

use hs_loop::repl::load_session;
use hs_loop::uipaint::UiEvent;
use std::sync::{Arc, Mutex};
use std::time::Instant;

fn write_fixture(dir: &std::path::Path) {
    std::fs::create_dir_all(dir).unwrap();
    let toml = r#"
[[tools]]
name = "answer.submit"
command = ["/mnt/instinct-nvme/hairspring/target/debug/hs-plugin-answersubmit"]
subjects = ["*"]

[[tools]]
name = "checker.run"
command = ["/mnt/instinct-nvme/hairspring/target/debug/hs-plugin-liechecker"]
subjects = ["*"]

[[tools]]
name = "agent.spawn"
command = ["/mnt/instinct-nvme/hairspring/target/debug/hs-plugin-swarm"]
subjects = ["*"]

[[tools]]
name = "agent.spawn_poll"
command = ["/mnt/instinct-nvme/hairspring/target/debug/hs-plugin-swarm", "--as", "agent.spawn_poll"]
subjects = ["*"]

[[models]]
name = "scripted-parent"
command = ["/bin/sh", "-c", "HS_SCRIPTED_NAME=scripted-parent HS_SEQMODEL_DELAY_MS=2000 exec /mnt/instinct-nvme/hairspring/target/debug/hs-plugin-scripted"]
default = true
subjects = ["*"]

[[models]]
name = "scripted-fast"
command = ["/bin/sh", "-c", "HS_SCRIPTED_NAME=scripted-fast HS_SEQMODEL_DELAY_MS=300 exec /mnt/instinct-nvme/hairspring/target/debug/hs-plugin-scripted"]
subjects = ["*"]

[[models]]
name = "scripted-slow"
command = ["/bin/sh", "-c", "HS_SCRIPTED_NAME=scripted-slow HS_SEQMODEL_DELAY_MS=2000 exec /mnt/instinct-nvme/hairspring/target/debug/hs-plugin-scripted"]
subjects = ["*"]
"#;
    std::fs::write(dir.join("hairspring.toml"), toml).unwrap();
    // Parent (2s/call pacing): spawn slow, spawn fast, one poll on a
    // bogus id that just buys a step, then prompt-aware submit.
    // Children replay from line 1 under their OWN name: two spawn
    // attempts (depth-refused), the bogus poll (unknown id), then
    // prompt-aware submit - 4 calls each.
    // Timeline: slow spawned @2s, done ~10s (4x2s). Fast spawned @4s,
    // done ~5.2s (4x0.3s). Step-3-boundary poll @6s sees the fast
    // finish -> step-4 prompt carries DELEGATION UPDATES. Parent's
    // submit @8s JOINS the slow child -> wall ~10s (serial ~17s).
    std::fs::write(
        dir.join("script.jsonl"),
        "{\"tool\":\"agent.spawn\",\"args\":{\"mission\":\"slow child\",\"model\":\"scripted-slow\"}}\n\
         {\"tool\":\"agent.spawn\",\"args\":{\"mission\":\"fast child\",\"model\":\"scripted-fast\"}}\n\
         {\"tool\":\"agent.spawn_poll\",\"args\":{\"child_stream_id\":\"00000000-0000-0000-0000-000000000000\"}}\n",
    )
    .unwrap();
}

fn stream_texts(log_root: &std::path::Path) -> std::collections::HashMap<String, String> {
    let mut out = std::collections::HashMap::new();
    for e in std::fs::read_dir(log_root.join("streams")).unwrap() {
        let name = e.unwrap().file_name().to_string_lossy().to_string();
        let sid = uuid::Uuid::parse_str(&name).unwrap();
        let reader = hs_log::StreamReader::open(log_root, sid).unwrap();
        let mut all = String::new();
        for ev in reader.events().unwrap() {
            all.push_str(&format!("{:?} ", ev.kind));
            if let Ok(b) = reader.resolve_payload(&ev) {
                all.push_str(&String::from_utf8_lossy(&b));
            }
            all.push('\n');
        }
        out.insert(name, all);
    }
    out
}

#[test]
fn r1_children_run_concurrently_and_join_at_close() {
    let dir = std::env::temp_dir().join("swarm-async-r1");
    let _ = std::fs::remove_dir_all(&dir);
    write_fixture(&dir);
    std::env::remove_var("HS_SWARM_DEPTH");
    std::env::set_var("HS_ANSWER_RAW", "1");
    std::env::set_var("HS_SCRIPTED_PROMPT_AWARE", "1");
    std::env::set_var("HS_SEQMODEL_SCRIPT", dir.join("script.jsonl"));
    std::env::set_var("HS_SWARM_LOG_ROOT", dir.join("run"));
    std::env::set_var("HS_SWARM_CONFIG", dir.join("hairspring.toml"));

    let events: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
    let cap = events.clone();
    let mut s = load_session(&dir.join("hairspring.toml"), &dir.join("run"), false, 12, None, None)
        .unwrap();
    s.set_ui_sink(Box::new(move |ev: UiEvent| {
        cap.lock().unwrap().push(format!("{ev:?}"));
    }));

    let t0 = Instant::now();
    let r = s.run_goal("delegate two children").unwrap();
    let wall = t0.elapsed();
    assert!(r.passed, "delegating parent passes: {r:?}");

    // CONCURRENCY: parent 4x2s + slow child 4x2s + fast child 4x0.3s
    // serial ~= 17s; concurrent + join ~= 10s. The lower bound proves
    // the passing close JOINED the still-running slow child instead
    // of closing early at the parent's own ~8s.
    assert!(
        wall.as_secs_f64() < 13.0,
        "children ran concurrently (wall {wall:?} << serial ~17s)"
    );
    assert!(
        wall.as_secs_f64() >= 8.5,
        "the slow child really paced the join (wall {wall:?})"
    );

    let ev = events.lock().unwrap();
    let spawned = ev.iter().filter(|e| e.starts_with("SubAgentSpawned")).count();
    let finished = ev.iter().filter(|e| e.starts_with("SubAgentFinished")).count();
    assert_eq!(spawned, 2, "both children raised SubAgentSpawned mid-mission: {ev:?}");
    assert_eq!(finished, 2, "both children raised SubAgentFinished: {ev:?}");
    // Both finishes are happy.
    assert!(
        ev.iter()
            .filter(|e| e.starts_with("SubAgentFinished"))
            .all(|e| e.contains("ok: true")),
        "both children passed: {ev:?}"
    );
    drop(ev);

    let streams = stream_texts(&dir.join("run"));
    // Parent mission stream: TWO Spawn events, each booked BEFORE its
    // ToolCall (atomic booking), delegation updates in the prompts,
    // and both depth refusals in the children.
    let parent = streams.get(&r.stream_id.to_string()).expect("parent stream");
    assert_eq!(parent.matches("Spawn ").count(), 2, "two Spawn bookings (exactly one per delegation)");
    let spawn_pos: Vec<usize> = parent.match_indices("Spawn ").map(|(i, _)| i).collect();
    let toolcall_pos: Vec<usize> = parent.match_indices("ToolCall ").map(|(i, _)| i).collect();
    assert!(
        spawn_pos.iter().zip(toolcall_pos.iter()).all(|(sp, tp)| sp < tp),
        "every Spawn precedes its ToolCall (booking-before-spawn)"
    );
    // Mid-run event paradigm: the fast child FINISHES while the
    // parent is still stepping (@5.2s vs parent's step 4 @6-8s), and
    // the next prompt carries its outcome - long before the passing
    // close joins the slow child.
    let upd_lines: Vec<&str> = parent
        .lines()
        .filter(|l| l.contains("DELEGATION UPDATES"))
        .collect();
    assert!(
        upd_lines.iter().any(|l| l.contains("fast child")),
        "the fast child's finish surfaced mid-run in a prompt: {upd_lines:?}"
    );
    // Cost honesty: parent books include both children's spend.
    assert!(
        r.cost_micros > 6_000,
        "parent cost folds BOTH children (8 paced child calls total): {} micros",
        r.cost_micros
    );
    // Children: each stream ran its mission, hit the depth guard, and
    // answered for real.
    let children: Vec<&String> = streams
        .iter()
        .filter(|(id, _)| *id != &r.stream_id.to_string())
        .map(|(_, v)| v)
        .filter(|v| v.contains("child_of"))
        .collect();
    assert_eq!(children.len(), 2, "two child streams with child_of provenance");
    for c in &children {
        assert!(c.contains("delegation depth limit"), "depth guard held in child");
        assert!(c.contains("scripted answer: mission complete"), "child answered");
    }
    // The disk registry: both reports written.
    let reports = std::fs::read_dir(dir.join("run/swarm"))
        .unwrap()
        .filter(|e| e.as_ref().unwrap().file_name().to_string_lossy().ends_with(".report.json"))
        .count();
    assert_eq!(reports, 2, "both completion reports on disk");
}
