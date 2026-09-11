//! RED: SwarmWorld-fidelity gap 1 - spawned children (agent.spawn) must
//! join BOTH shared planes: the typed memory plane K (memory.recall served,
//! close-time distillation into the shared K) and the world plane (world.*
//! tools attached, proposals visible to other sessions). Today
//! hs-swarm builds children with tb_tools + agent.spawn ONLY - no memory
//! db, no world - so every plane assertion here fails pre-fix.
//! (Plugin binaries come from the workspace debug build, same pattern as
//! hs-loop/tests/swarm_async_red.rs.)

use hs_core::EventKind;
use hs_memory::MemoryStore;

const ANSWER: &str = "/mnt/instinct-nvme/hairspring/target/debug/hs-plugin-answer";
const CHECKER: &str = "/mnt/instinct-nvme/hairspring/target/debug/hs-plugin-checker";
const SCRIPTED: &str = "/mnt/instinct-nvme/hairspring/target/debug/hs-plugin-scripted";

const MARKER: &str = "PARENT-K-MARKER-ALPHA-SEP";

fn payloads(log: &std::path::Path, stream: uuid::Uuid, kind: EventKind) -> Vec<String> {
    let reader = hs_log::StreamReader::open(log, stream).unwrap();
    reader
        .events()
        .unwrap()
        .iter()
        .filter(|e| e.kind == kind)
        .filter_map(|e| reader.resolve_payload(e).ok())
        .map(|b| String::from_utf8_lossy(&b).into_owned())
        .collect()
}

#[test]
fn spawned_child_joins_memory_and_world_planes() {
    let dir = std::env::temp_dir().join(format!("hsplanes-g1-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let log = dir.join("log");
    std::fs::create_dir_all(&log).unwrap();

    // Seed the shared K so the child's recall has a parent-authored record
    // to find.
    let db = log.join("memory.db");
    {
        let store = hs_memory::sqlite::SqliteMemoryStore::open(&db).unwrap();
        store
            .put(hs_memory::NewMemoryRecord {
                agent_id: "operator".to_string(),
                mission_id: Some("parent-mission".to_string()),
                kind: "semantic".to_string(),
                content: MARKER.to_string(),
                importance: 0.95,
                expires_at: None,
                source_seqs: vec![9],
            })
            .unwrap();
    }

    let mission = "task-7";
    let answer = log.join("work").join(mission).join("answer.txt");
    let script = dir.join("child.jsonl");
    std::fs::write(
        &script,
        [
            serde_json::json!({"tool":"memory.recall","args":{"k":3}}),
            serde_json::json!({"tool":"world.observe","args":{"world_path":"/skills"}}),
            serde_json::json!({"tool":"world.propose","args":{"world_path":"/skills/child-skill","kind":"skill","content":"the child learned a reusable move"}}),
            serde_json::json!({"tool":"answer.write","args":{"path": answer.display().to_string(), "content":"TOKEN-7-SECRET"}}),
        ]
        .iter()
        .map(std::string::ToString::to_string)
        .collect::<Vec<_>>()
        .join("\n"),
    )
    .unwrap();
    let config = dir.join("hairspring.toml");
    std::fs::write(
        &config,
        format!(
            r#"
[[tools]]
name = "answer.write"
command = ["{ANSWER}"]
subjects = ["*"]

[[tools]]
name = "checker.run"
command = ["{CHECKER}"]
subjects = ["*"]

[[models]]
name = "child"
command = ["/bin/sh", "-c", "HS_SCRIPTED_NAME=child HS_SEQMODEL_SCRIPT={} exec {SCRIPTED}"]
default = true
subjects = ["*"]
"#,
            script.display()
        ),
    )
    .unwrap();

    let spawner = hs_swarm::Spawner::new(&log, &config, false, 10);
    let parent_stream = uuid::Uuid::new_v4();
    hs_log::StreamWriter::create(&log, parent_stream).unwrap();
    let (child, _delegation_ms) = spawner.spawn(&log, parent_stream, mission).unwrap();
    let report = spawner.run_to_completion(&child).unwrap();
    assert!(report.passed, "child mission should pass: {report:?}");

    let calls = payloads(&log, child.stream_id, EventKind::ToolCall);

    // Plane K: recall OFFERED (not an unknown-tool error) and SERVED the
    // parent-seeded record from the shared memory.db.
    let recalls: Vec<&String> = calls.iter().filter(|c| c.contains("memory.recall")).collect();
    assert!(
        !recalls.is_empty(),
        "child never got memory.recall served: {calls:?}"
    );
    assert!(
        recalls.iter().any(|c| c.contains(MARKER)),
        "child recall did not see the shared K marker: {recalls:?}"
    );

    // Plane world: observe + propose attached and served (no
    // "no world attached" errors) ...
    let observes: Vec<&String> = calls.iter().filter(|c| c.contains("world.observe")).collect();
    assert!(
        !observes.is_empty() && observes.iter().all(|c| !c.contains("no world attached")),
        "world.observe not served to child: {observes:?}"
    );
    // ... and the child proposal is observable in the shared world by a
    // FRESH world handle (cross-session stigmergy shape).
    let world = hs_world::World::open(&log).unwrap();
    let arts = world.observe("/skills/child-skill").unwrap();
    assert_eq!(
        arts.len(),
        1,
        "child-proposed artifact missing from the shared world: {arts:?}"
    );

    // Close-time distillation from the CHILD: a record with the child
    // mission id lands in the SHARED memory.db.
    let store = hs_memory::sqlite::SqliteMemoryStore::open(&db).unwrap();
    let hits = store.top_k("operator", 50).unwrap();
    assert!(
        hits.iter().any(|r| r.mission_id.as_deref() == Some(mission)),
        "child close did not distill into the shared K: {hits:?}"
    );

    let _ = std::fs::remove_dir_all(&dir);
}
