//! An omitted child step cap must not silently become a 16-step budget.
use hs_swarm::Spawner;

#[test]
fn unbounded_child_spawn_marker_has_no_step_cap() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("log");
    let parent = uuid::Uuid::new_v4();
    hs_log::StreamWriter::create(&root, parent).unwrap();
    let spawner = Spawner::unbounded(&root, &tmp.path().join("unused.toml"), false);
    let (_child, _) = spawner.spawn(&root, parent, "uncapped task").unwrap();
    let events = hs_log::StreamReader::open(&root, parent).unwrap().events().unwrap();
    let reader = hs_log::StreamReader::open(&root, parent).unwrap();
    let spawned = events.iter().find(|e| e.kind == hs_core::EventKind::Spawn).unwrap();
    let payload: serde_json::Value = serde_json::from_slice(&reader.resolve_payload(spawned).unwrap()).unwrap();
    assert!(payload["budget"]["max_steps"].is_null(), "uncapped child gained a cap: {payload}");
}
