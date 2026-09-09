//! RED-first (checklist 7.5, consumption half): the loop's predictor
//! retirement knobs are policy, not constants - a session given a
//! promoted `PrefetchPolicy` retires by ITS `min_samples`/crossover,
//! not by the compiled-in defaults. Pinned against the k3 fixture:
//! 5 distinct recalls, every prefetch a miss; the default predictor
//! retires on the 4th resolution, the overridden one on the 2nd.

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

#[test]
fn v3_promoted_knobs_drive_retirement_not_the_constants() {
    let _guard = ENV_LOCK.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let dir = tempfile::tempdir().unwrap();
    let log = tempfile::tempdir().unwrap();
    let db = tempfile::tempdir().unwrap().keep().join("memory.db");
    seed(&db);
    let path = log
        .path()
        .join("work")
        .join("task-0")
        .join("answer.txt")
        .display()
        .to_string();
    let script = write_script(
        dir.path(),
        &[
            serde_json::json!({"tool":"memory.recall","args":{"k":1}}),
            serde_json::json!({"tool":"memory.recall","args":{"k":2}}),
            serde_json::json!({"tool":"memory.recall","args":{"k":3}}),
            serde_json::json!({"tool":"memory.recall","args":{"k":4}}),
            serde_json::json!({"tool":"memory.recall","args":{"k":5}}),
            serde_json::json!({"tool":"answer.write","args":{"path":path,"content":"TOKEN-0-SECRET"}}),
        ],
    );
    unsafe { std::env::set_var("HS_SEQMODEL_SCRIPT", &script) };
    let mut l = rig(dir.path(), log.path(), 10);
    l.set_memory_db(&db);
    // the promoted policy: retire after 2 resolutions at 50% crossover
    l.set_prefetch_knobs(2, 5_000);
    let stream = l.stream_id();
    let r = l.run_mission("task-0").unwrap();
    assert!(r.passed, "mission passes: {r:?}");
    let prefetch = payloads(log.path(), stream, EventKind::Prefetch);
    assert_eq!(
        prefetch.len(),
        2,
        "the overridden predictor retires on the 2nd resolution: {prefetch:?}"
    );
    assert!(
        prefetch[1].contains("\"predictor_retired\":true"),
        "retirement booked with the promoted knobs visible: {}",
        prefetch[1]
    );
    assert!(
        prefetch[1].contains("\"min_samples\":2"),
        "the booking names the knob that fired: {}",
        prefetch[1]
    );
}
