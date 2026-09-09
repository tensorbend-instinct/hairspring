//! Eric's five #5 (2026-09-08): live multi-agent delegation. hs-swarm
//! is a complete delegation engine (Spawner::spawn writes a Spawn
//! event on the parent stream and creates the child's stream;
//! run_to_completion runs the child mission on its own stream) with
//! ZERO callers: no agent.spawn tool exists, and the TUI's delegation
//! graph consumes UiEvent::SubAgentSpawned/Finished which nothing
//! emits. This test wires the path end to end through the ONE legal
//! shape: a tool the model calls.
//!
//! Contract: a mission whose model calls agent.spawn {mission}
//! delegates a child mission on the same substrate - the child runs
//! for real (own stream, own answer), the parent stream records the
//! Spawn link, the ui_sink sees SubAgentSpawned AND SubAgentFinished
//! (the :agents panel's live feed), and the child's cost lands in the
//! parent's books so the $ guard stays honest across delegation.

use hs_loop::repl::load_session;
use hs_loop::uipaint::UiEvent;
use std::sync::{Arc, Mutex};

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

[[models]]
name = "scripted"
command = ["/mnt/instinct-nvme/hairspring/target/debug/hs-plugin-scripted"]
default = true
subjects = ["*"]
"#;
    std::fs::write(dir.join("hairspring.toml"), toml).unwrap();
    // Call 1: delegate. Everything after: prompt-aware fallback.
    std::fs::write(
        dir.join("script.jsonl"),
        "{\"tool\":\"agent.spawn\",\"args\":{\"mission\":\"child task\"}}\n",
    )
    .unwrap();
}

/// Every payload in every stream under the run dir, per stream id.
fn ledger_by_stream(
    log_root: &std::path::Path,
) -> std::collections::HashMap<String, String> {
    let streams = log_root.join("streams");
    let mut out = std::collections::HashMap::new();
    for e in std::fs::read_dir(&streams).unwrap() {
        let name = e.unwrap().file_name().to_string_lossy().to_string();
        let sid = uuid::Uuid::parse_str(&name).unwrap();
        let reader = hs_log::StreamReader::open(log_root, sid).unwrap();
        let mut all = String::new();
        for ev in reader.events().unwrap() {
            // Kind travels too: a Spawn booking is the kind, not the payload.
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
fn r1_delegation_runs_a_real_child_and_books_it() {
    std::env::set_var("HS_ANSWER_RAW", "1");
    std::env::set_var("HS_SCRIPTED_PROMPT_AWARE", "1");
    let dir = std::env::temp_dir().join("swarm-live-r1");
    let _ = std::fs::remove_dir_all(&dir);
    write_fixture(&dir);
    // The test process is the root of the delegation tree.
    std::env::remove_var("HS_SWARM_DEPTH");
    std::env::set_var("HS_SEQMODEL_SCRIPT", dir.join("script.jsonl"));
    std::env::set_var("HS_SWARM_LOG_ROOT", dir.join("run"));
    std::env::set_var("HS_SWARM_CONFIG", dir.join("hairspring.toml"));

    let events: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
    let cap = events.clone();
    let mut s = load_session(&dir.join("hairspring.toml"), &dir.join("run"), false, 8, None, None)
        .unwrap();
    s.set_ui_sink(Box::new(move |ev: UiEvent| {
        cap.lock().unwrap().push(format!("{ev:?}"));
    }));
    let r = s.run_goal("parent task").unwrap();
    assert!(r.passed, "delegating parent passes: {r:?}");

    // Four streams: parent mission + parent kernel dispatch + child
    // mission + child kernel dispatch.
    let ledgers = ledger_by_stream(&dir.join("run"));
    assert_eq!(
        ledgers.len(),
        4,
        "parent+child mission streams plus both kernel dispatch streams: {:?}",
        ledgers.keys()
    );

    let parent = ledgers.get(&r.stream_id.to_string()).expect("parent stream");
    // The parent stream records the Spawn event naming the child AND
    // the child's model (delegation provenance, spawner-reported).
    assert!(parent.contains("Spawn"), "parent books the Spawn event");
    assert!(
        parent.contains("\"model\":\"scripted\""),
        "Spawn payload carries the child's model"
    );
    let child_id = {
        const KEY: &str = "\"child_stream_id\":\"";
        let m = parent.find(KEY).expect("Spawn names the child") + KEY.len();
        let tail = &parent[m..];
        tail[..tail.find('"').expect("uuid is quoted")].to_string()
    };
    let child = ledgers.get(&child_id).expect("child stream exists");
    // The child really ran: its stream names the mission, its answer
    // file exists with real content.
    assert!(child.contains("child task"), "child stream carries its mission");
    // hs-swarm runs the mission verbatim (no goal slugging).
    let child_answer = dir.join("run/work/child task/answer.txt");
    assert_eq!(
        std::fs::read_to_string(&child_answer).unwrap(),
        "scripted answer: mission complete",
        "child wrote its own answer"
    );

    // The child tried to delegate (same script line 1) and the depth
    // guard refused it - the fork-bomb guard is pinned by the child's
    // own feedback beat.
    assert!(
        child.contains("delegation depth limit"),
        "depth guard refused the grandchild: {}",
        &child[..child.len().min(400)]
    );

    // The UI feed: both delegation events reached the sink.
    let evs = events.lock().unwrap();
    assert!(
        evs.iter().any(|e| e.starts_with("SubAgentSpawned")),
        "ui sink saw SubAgentSpawned: {evs:?}"
    );
    assert!(
        evs.iter().any(|e| e.starts_with("SubAgentFinished")),
        "ui sink saw SubAgentFinished: {evs:?}"
    );

    // The child's cost is in the parent's books. A non-delegating
    // scripted mission books 3500 micros (fixture_honesty_red); the
    // child adds its own calls on top.
    assert!(
        r.cost_micros > 3500,
        "parent cost includes the child's calls: {}",
        r.cost_micros
    );
}
