//! RED-first (B1, v5 D3 + cut #10): the K memory plane is a tool the model
//! CONSULTS (`memory.recall`), never a mandatory retrieval pre-pass;
//! retrieval is prefetched ahead of the next model call, every prefetch is
//! booked as a `Prefetch` event with its hit/miss outcome and token cost,
//! and the predictor retires itself (with the event booked) when the hit
//! rate falls below the logging-cost crossover.
//!
//! k1 schema shape + serving + no-store clean error + no pre-pass markers
//! k2 same-args recall twice -> a Prefetch event with hit:true + token cost
//! k3 distinct recalls -> predictor retires (4 resolutions; 0 prefetch after)

use hs_core::EventKind;
use hs_loop::*;
use hs_memory::{MemoryStore, NewMemoryRecord};

static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

const ANSWER: &str = env!("CARGO_BIN_EXE_hs-plugin-answer");
const CHECKER: &str = env!("CARGO_BIN_EXE_hs-plugin-checker");
const SCRIPTED: &str = env!("CARGO_BIN_EXE_hs-plugin-scripted");

const MARKER: &str = "KMARKER-7G-FIB(17)=1597-SEPARATOR";

fn rig(dir: &std::path::Path, log: &std::path::Path, max_steps: u32) -> InnerLoop {
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
name = "scripted"
command = ["{SCRIPTED}"]
default = true
"#
        ),
    )
    .unwrap();
    let kernel = hs_kernel::Kernel::load(&config).unwrap();
    InnerLoop::new(kernel, log, true, max_steps).unwrap()
}

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

fn seed(db: &std::path::Path) {
    let store = hs_memory::sqlite::SqliteMemoryStore::open(db).unwrap();
    store
        .put(NewMemoryRecord {
            agent_id: "operator".to_string(),
            mission_id: Some("seed-mission".to_string()),
            kind: "semantic".to_string(),
            content: MARKER.to_string(),
            importance: 0.95,
            expires_at: None,
            source_seqs: vec![9],
        })
        .unwrap();
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

fn answer_line(log: &std::path::Path) -> serde_json::Value {
    let path = log
        .join("work")
        .join("task-0")
        .join("answer.txt")
        .display()
        .to_string();
    serde_json::json!({"tool":"answer.write","args":{"path":path,"content":"TOKEN-0-SECRET"}})
}

fn run(rig_dir: &std::path::Path, log: &std::path::Path, db: Option<&std::path::Path>, script_lines: &[serde_json::Value], max_steps: u32) -> uuid::Uuid {
    let script = write_script(rig_dir, script_lines);
    unsafe { std::env::set_var("HS_SEQMODEL_SCRIPT", &script) };
    let mut l = rig(rig_dir, log, max_steps);
    if let Some(db) = db {
        l.set_memory_db(db);
    }
    let stream = l.stream_id();
    let r = l.run_mission("task-0").unwrap();
    assert!(r.passed, "mission passes: {r:?}");
    stream
}

#[test]
fn k1_memory_recall_serves_k_records_to_the_model() {
    let _guard = ENV_LOCK.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let dir = tempfile::tempdir().unwrap();
    let log = tempfile::tempdir().unwrap();
    let db = tempfile::tempdir().unwrap().keep().join("memory.db");
    seed(&db);
    let stream = run(
        dir.path(),
        log.path(),
        Some(&db),
        &[
            serde_json::json!({"tool":"memory.recall","args":{"k":3}}),
            answer_line(log.path()),
        ],
        6,
    );
    let calls = payloads(log.path(), stream, EventKind::ToolCall);
    let recall = calls
        .iter()
        .find(|p| p.contains("\"plugin\":\"memory.recall\""))
        .expect("the memory.recall tool call was dispatched");
    assert!(recall.contains(MARKER), "K served the seeded record: {recall}");
    assert!(
        recall.contains("\"source_seqs\":["),
        "provenance rides the tool result: {recall}"
    );
    // the result reaches the model's next turn (transcript replay of the
    // ToolCall event), the whole point of consulting K
    let prompts = payloads(log.path(), stream, EventKind::ModelCall);
    assert!(
        prompts.iter().any(|p| p.contains(MARKER)),
        "memory result visible to the model"
    );
    // cut #10: with K consulted by tool, NOTHING memory-shaped is forced
    // into a prompt - the removed mandatory pre-pass stays removed.
    assert!(
        prompts.iter().all(|p| !p.contains("MEMORY (earlier missions)")),
        "no mandatory memory pre-pass survives"
    );
}

#[test]
fn k1b_memory_recall_without_store_is_a_clean_error() {
    let _guard = ENV_LOCK.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let dir = tempfile::tempdir().unwrap();
    let log = tempfile::tempdir().unwrap();
    let stream = run(
        dir.path(),
        log.path(),
        None,
        &[
            serde_json::json!({"tool":"memory.recall","args":{"k":3}}),
            answer_line(log.path()),
        ],
        6,
    );
    let calls = payloads(log.path(), stream, EventKind::ToolCall);
    let recall = calls
        .iter()
        .find(|p| p.contains("\"plugin\":\"memory.recall\""))
        .expect("the memory.recall tool call was dispatched");
    assert!(
        recall.contains("no memory store"),
        "explicit, non-fatal error when K is unattached: {recall}"
    );
}

#[test]
fn k2_prefetch_booked_with_hit_and_token_cost() {
    let _guard = ENV_LOCK.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let dir = tempfile::tempdir().unwrap();
    let log = tempfile::tempdir().unwrap();
    let db = tempfile::tempdir().unwrap().keep().join("memory.db");
    seed(&db);
    let stream = run(
        dir.path(),
        log.path(),
        Some(&db),
        &[
            serde_json::json!({"tool":"memory.recall","args":{"k":3}}),
            serde_json::json!({"tool":"memory.recall","args":{"k":3}}),
            answer_line(log.path()),
        ],
        8,
    );
    let prefetch = payloads(log.path(), stream, EventKind::Prefetch);
    let hit = prefetch
        .iter()
        .find(|p| p.contains("\"hit\":true"))
        .expect("the second identical recall resolves the prefetch as a HIT");
    assert!(hit.contains("\"predicted\":"), "the speculation is named: {hit}");
    assert!(hit.contains("\"tokens_est\":"), "its token cost is booked: {hit}");
    assert!(
        !hit.contains("\"tokens_est\":0"),
        "the cost is nonzero: {hit}"
    );
}

#[test]
fn k3_predictor_retires_below_the_cost_crossover() {
    let _guard = ENV_LOCK.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let dir = tempfile::tempdir().unwrap();
    let log = tempfile::tempdir().unwrap();
    let db = tempfile::tempdir().unwrap().keep().join("memory.db");
    seed(&db);
    let stream = run(
        dir.path(),
        log.path(),
        Some(&db),
        &[
            serde_json::json!({"tool":"memory.recall","args":{"k":1}}),
            serde_json::json!({"tool":"memory.recall","args":{"k":2}}),
            serde_json::json!({"tool":"memory.recall","args":{"k":3}}),
            serde_json::json!({"tool":"memory.recall","args":{"k":4}}),
            serde_json::json!({"tool":"memory.recall","args":{"k":5}}),
            answer_line(log.path()),
        ],
        10,
    );
    let prefetch = payloads(log.path(), stream, EventKind::Prefetch);
    assert_eq!(
        prefetch.len(),
        4,
        "exactly the 4 resolutions; the retired predictor fetches nothing more: {prefetch:?}"
    );
    assert!(
        prefetch.iter().all(|p| p.contains("\"hit\":false")),
        "5 distinct recalls = 4 misses: {prefetch:?}"
    );
    assert!(
        prefetch[3].contains("\"predictor_retired\":true"),
        "retirement booked, tunable crossover visible: {}",
        prefetch[3]
    );
}

#[test]
fn k4_memory_recall_schema_shape() {
    let t = hs_loop::toolschema::memory_recall_tool();
    assert_eq!(t["type"].as_str(), Some("function"));
    assert_eq!(
        t["function"]["name"].as_str(),
        Some("memory.recall"),
        "K is consulted as memory.recall"
    );
    assert_eq!(
        t["function"]["parameters"]["type"].as_str(),
        Some("object"),
        "schema parameters object"
    );
    assert!(
        t["function"]["description"]
            .as_str()
            .unwrap_or("")
            .len()
            > 20,
        "the schema carries its behavioral description"
    );
}

#[test]
fn k6_within_session_distillation_is_mission_scoped() {
    let _guard = ENV_LOCK.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    // B1 close-distillation defect (live TUI proof, 2026-09-09): extraction
    // read the WHOLE shared session stream, so mission N's record
    // inherited mission N-1's edits and provenance. Within ONE session,
    // each close must distill ONLY that mission's events.
    let dir = tempfile::tempdir().unwrap();
    let log = tempfile::tempdir().unwrap();
    let db = tempfile::tempdir().unwrap().keep().join("memory.db");
    // one plugin process, one session: the script freezes at launch, so
    // both missions' lines live in one file, consumed in order.
    let a0 = format!("{}/work/task-0/answer.txt", log.path().display());
    let a1 = format!("{}/work/task-1/answer.txt", log.path().display());
    let script = write_script(
        dir.path(),
        &[
            serde_json::json!({"tool":"answer.write","args":{"path":a0,"content":"TOKEN-0-SECRET"}}),
            serde_json::json!({"tool":"answer.write","args":{"path":a1,"content":"TOKEN-1-SECRET"}}),
        ],
    );
    unsafe { std::env::set_var("HS_SEQMODEL_SCRIPT", &script) };
    let mut l = rig(dir.path(), log.path(), 4);
    l.set_memory_db(&db);
    let r1 = l.run_mission("task-0").unwrap();
    assert!(r1.passed, "mission 1 passes: {r1:?}");
    let r2 = l.run_mission("task-1").unwrap();
    assert!(r2.passed, "mission 2 passes: {r2:?}");

    let store = hs_memory::sqlite::SqliteMemoryStore::open(&db).unwrap();
    let all = store.top_k("operator", 10).unwrap();
    let m1 = all
        .iter()
        .find(|r| r.mission_id.as_deref() == Some("task-0"))
        .expect("mission 1's close distilled a record");
    let m2 = all
        .iter()
        .find(|r| r.mission_id.as_deref() == Some("task-1"))
        .expect("mission 2's close distilled a record");
    let overlap: Vec<u64> = m2
        .source_seqs
        .iter()
        .filter(|s| m1.source_seqs.contains(s))
        .copied()
        .collect();
    assert!(
        overlap.is_empty(),
        "mission 2's record must not borrow mission 1's events: {overlap:?} (m2: {m2:?})"
    );
    assert!(
        m2.content.contains("task-1/answer.txt"),
        "m2's edits name its own answer: {m2:?}"
    );
    assert!(
        !m2.content.contains("task-0/answer.txt"),
        "m2 must not inherit m1's edits: {}",
        m2.content
    );
    assert!(
        m1.content.contains("task-0/answer.txt"),
        "m1's edits name its own answer: {m1:?}"
    );
}
