//! RED: SwarmWorld-fidelity gap closure (Eric 2026-09-11, relayed by main:
//! "Fix all three faithfully with UT/TDD to make sure it works correctly
//! and measure its success empirically"):
//!   G1 - spawned children join BOTH shared planes (K + world); the test
//!        lives in hs-swarm/tests/swarm_planes_red.rs (Spawner is hs-swarm's
//!        API; hs-loop cannot dev-depend on it).
//!   G2 - close-time distillation also flows INTO the world: each distilled
//!        record is proposed as a world artifact (procedural -> skill,
//!        episodic -> note) landing VALIDATED - never auto-installed (the
//!        world service + explicit world.install is the assay gate; spec's
//!        promotion machinery is future work).
//!   G3 - reuse bookkeeping at the loop dispatch: world.propose accepts
//!        parent_version from the model and world.observe books reuse and
//!        reports reuse_count. (World-level round-trip lives in
//!        hs-world/tests/reuse_red.rs.)

use hs_core::EventKind;
use hs_memory::MemoryStore;

const ANSWER: &str = env!("CARGO_BIN_EXE_hs-plugin-answer");
const CHECKER: &str = env!("CARGO_BIN_EXE_hs-plugin-checker");
const SCRIPTED: &str = env!("CARGO_BIN_EXE_hs-plugin-scripted");

fn write_script(dir: &std::path::Path, lines: &[serde_json::Value]) -> std::path::PathBuf {
    let p = dir.join("script.jsonl");
    std::fs::write(
        &p,
        lines
            .iter()
            .map(std::string::ToString::to_string)
            .collect::<Vec<_>>()
            .join("\n"),
    )
    .unwrap();
    p
}

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

fn config_with_scripted(
    dir: &std::path::Path,
    model: &str,
    script: &std::path::Path,
) -> std::path::PathBuf {
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
name = "{model}"
command = ["/bin/sh", "-c", "HS_SCRIPTED_NAME={model} HS_SEQMODEL_SCRIPT={} exec {SCRIPTED}"]
default = true
subjects = ["*"]
"#,
            script.display()
        ),
    )
    .unwrap();
    config
}

/// G2: close-time distillation flows into the world as VALIDATED
/// (never auto-installed) artifacts - procedural records become skills,
/// episodic records become notes.
#[test]
fn g2_close_distills_into_world_as_validated_not_installed() {
    let dir = std::env::temp_dir().join(format!("hsplanes-g2-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let log = dir.join("log");
    std::fs::create_dir_all(&log).unwrap();
    let db = log.join("memory.db");

    // A FAILING mission: the scripted model keeps writing the wrong token,
    // the ground-truth checker keeps failing, the close distills an
    // episodic record AND a procedural learned-from-failure record.
    let mission = "task-3";
    let answer = log.join("work").join(mission).join("answer.txt");
    let script = write_script(
        &dir,
        &[serde_json::json!({"tool":"answer.write","args":{"path": answer.display().to_string(), "content":"WRONG-TOKEN"}})],
    );
    let config = config_with_scripted(&dir, "scripted", &script);

    let kernel = hs_kernel::Kernel::load(&config).unwrap();
    let mut l = hs_loop::InnerLoop::new(kernel, &log, true, 4).unwrap();
    l.set_memory_db(&db);
    l.attach_world();
    let stream = l.stream_id();
    let r = l.run_mission(mission).unwrap();
    assert!(!r.passed, "mission with wrong token must fail: {r:?}");

    // The distiller still wrote K (existing behavior, kept) ...
    let store = hs_memory::sqlite::SqliteMemoryStore::open(&db).unwrap();
    let hits = store.top_k("operator", 50).unwrap();
    assert!(
        hits.iter()
            .any(|r| r.mission_id.as_deref() == Some(mission) && r.kind == "procedural"),
        "procedural record missing from K: {hits:?}"
    );

    // ... AND proposed the distilled records into the world:
    let world = hs_world::World::open(&log).unwrap();
    let skills = world.observe("/skills/task-3").unwrap();
    assert!(
        skills.iter().any(|a| a.kind == hs_world::ArtifactKind::Skill
            && a.author_stream == stream
            && a.status == hs_world::ArtifactStatus::Validated),
        "no Validated skill artifact from the distiller: {skills:?}"
    );
    let notes = world.observe("/knowledge/task-3").unwrap();
    assert!(
        notes.iter().any(|a| a.kind == hs_world::ArtifactKind::Note
            && a.author_stream == stream
            && a.status == hs_world::ArtifactStatus::Validated),
        "no Validated note artifact from the distiller: {notes:?}"
    );
    // The assay gate: the loop must NEVER auto-install - installation
    // stays an explicit world.install decision.
    assert!(
        skills.iter().chain(&notes)
            .all(|a| a.status == hs_world::ArtifactStatus::Validated),
        "distiller artifacts must land Validated, never auto-installed: {skills:?} {notes:?}"
    );

    let _ = std::fs::remove_dir_all(&dir);
}

/// G3 (loop-dispatch half): world.propose accepts parent_version from the
/// model and world.observe books reuse and reports reuse_count.
#[test]
fn g3_dispatch_carries_parent_version_and_reuse_count() {
    let dir = std::env::temp_dir().join(format!("hsplanes-g3-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let log = dir.join("log");
    std::fs::create_dir_all(&log).unwrap();

    // Session 1 proposes the base artifact directly through the world.
    let world = hs_world::World::open(&log).unwrap();
    let s1 = uuid::Uuid::new_v4();
    hs_log::StreamWriter::create(&log, s1).unwrap();
    let content = b"the reusable move";
    let hash: [u8; 32] = {
        use sha2::Digest;
        sha2::Sha256::digest(content).into()
    };
    let base = hs_world::Artifact {
        artifact_id: uuid::Uuid::new_v4(),
        version: 1,
        kind: hs_world::ArtifactKind::Skill,
        content_hash: hash,
        world_path: "/skills/base".to_string(),
        author_stream: s1,
        parent_version: None,
        status: hs_world::ArtifactStatus::Proposed,
    };
    let base = world.propose(base, content).unwrap();

    // Session 2 (a scripted loop with the world attached) observes it
    // twice - booking reuse - then proposes a MUTATION carrying
    // parent_version.
    let mission = "task-5";
    let answer = log.join("work").join(mission).join("answer.txt");
    let script = write_script(
        &dir,
        &[
            serde_json::json!({"tool":"world.observe","args":{"world_path":"/skills/base"}}),
            serde_json::json!({"tool":"world.observe","args":{"world_path":"/skills/base"}}),
            serde_json::json!({"tool":"world.propose","args":{"world_path":"/skills/mutated","kind":"skill","content":"the move, adapted","parent_version": base.artifact_id.to_string()}}),
            serde_json::json!({"tool":"answer.write","args":{"path": answer.display().to_string(), "content":"TOKEN-5-SECRET"}}),
        ],
    );
    let config = config_with_scripted(&dir, "scripted", &script);
    let kernel = hs_kernel::Kernel::load(&config).unwrap();
    let mut l = hs_loop::InnerLoop::new(kernel, &log, true, 10).unwrap();
    l.attach_world();
    let s2 = l.stream_id();
    let r = l.run_mission(mission).unwrap();
    assert!(r.passed, "mutation mission should pass: {r:?}");

    // Reuse was booked onto the world stream by the OBSERVING stream.
    let world2 = hs_world::World::open(&log).unwrap();
    assert_eq!(
        world2.reuse_count(base.artifact_id, 1),
        2,
        "two observes by session 2 must book two reuse events"
    );
    let obs = payloads(&log, hs_world::world_stream_id(), EventKind::Observation);
    let reuses: Vec<&String> = obs.iter().filter(|o| o.contains("\"reuse\"")).collect();
    assert!(
        reuses
            .iter()
            .any(|o| o.contains(&base.artifact_id.to_string()) && o.contains(&s2.to_string())),
        "no reuse event booking observer stream {s2} on artifact {}: {obs:?}",
        base.artifact_id
    );

    // The mutation proposal recorded its parent_version.
    let arts = world2.observe("/skills/mutated").unwrap();
    assert_eq!(arts.len(), 1, "mutation artifact missing: {arts:?}");
    assert_eq!(
        arts[0].parent_version,
        Some(base.artifact_id),
        "parent_version did not survive the dispatch: {:?}",
        arts[0]
    );

    // The observe output the MODEL saw reports reuse_count.
    let calls = payloads(&log, s2, EventKind::ToolCall);
    let observes: Vec<&String> = calls.iter().filter(|c| c.contains("world.observe")).collect();
    assert!(
        observes
            .iter()
            .any(|c| c.contains("\"reuse_count\":1") || c.contains("\"reuse_count\": 1")),
        "observe output carried no reuse_count: {observes:?}"
    );

    let _ = std::fs::remove_dir_all(&dir);
}
