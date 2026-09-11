//! RED: RMM retrospective mechanism (arXiv 2503.08026; Eric 2026-09-11:
//! "get RMM working"). memory.recall SERVES records; at mission close the
//! loop scores each served record retrospectively: its id cited in a later
//! model completion = +1, retrieved-but-never-cited = -1. The ledger lives
//! in K (append-only), per-record usefulness is the running sum, and the
//! close books the deltas onto the mission stream.

use hs_core::EventKind;
use hs_memory::MemoryStore;
use std::sync::Mutex;

/// HS_SEQMODEL_SCRIPT is process-global: these tests serialize.
static ENV_LOCK: Mutex<()> = Mutex::new(());

const ANSWER: &str = env!("CARGO_BIN_EXE_hs-plugin-answer");
const CHECKER: &str = env!("CARGO_BIN_EXE_hs-plugin-checker");
const SCRIPTED: &str = env!("CARGO_BIN_EXE_hs-plugin-scripted");

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
fn recall_citations_score_the_ledger_at_close() {
    let _g = ENV_LOCK.lock().unwrap();
    let dir = std::env::temp_dir().join(format!("hsrmm-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let log = dir.join("log");
    std::fs::create_dir_all(&log).unwrap();
    let db = log.join("memory.db");

    // Two records in K: the mission will cite ONE of them.
    let store = hs_memory::sqlite::SqliteMemoryStore::open(&db).unwrap();
    let id_cited = store
        .put(hs_memory::NewMemoryRecord {
            agent_id: "operator".to_string(),
            mission_id: Some("earlier-win".to_string()),
            kind: "semantic".to_string(),
            content: "RMM-USEFUL-FACT-A".to_string(),
            importance: 0.9,
            expires_at: None,
            source_seqs: vec![3],
        })
        .unwrap();
    let id_ignored = store
        .put(hs_memory::NewMemoryRecord {
            agent_id: "operator".to_string(),
            mission_id: Some("earlier-noise".to_string()),
            kind: "semantic".to_string(),
            content: "RMM-IGNORED-FACT-B".to_string(),
            importance: 0.8,
            expires_at: None,
            source_seqs: vec![4],
        })
        .unwrap();
    drop(store);

    let mission = "task-2";
    let answer = log.join("work").join(mission).join("answer.txt");
    let citation = log.join("work").join(mission).join("notes.txt");
    let script = dir.join("script.jsonl");
    std::fs::write(
        &script,
        [
            serde_json::json!({"tool":"memory.recall","args":{"k":5}}),
            // The model visibly cites the record it actually used ...
            serde_json::json!({"tool":"answer.write","args":{"path": citation.display().to_string(), "content": format!("the decisive prior fact was {id_cited}")}}),
            serde_json::json!({"tool":"answer.write","args":{"path": answer.display().to_string(), "content":"TOKEN-2-SECRET"}}),
        ]
        .iter()
        .map(std::string::ToString::to_string)
        .collect::<Vec<_>>()
        .join("\n"),
    )
    .unwrap();
    unsafe { std::env::set_var("HS_SEQMODEL_SCRIPT", &script) };

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
    let mut l = hs_loop::InnerLoop::new(kernel, &log, true, 8).unwrap();
    l.set_memory_db(&db);
    let stream = l.stream_id();
    let r = l.run_mission(mission).unwrap();
    assert!(r.passed, "mission should pass: {r:?}");

    // The ledger: cited +1, retrieved-but-uncited -1, readable per record.
    let store = hs_memory::sqlite::SqliteMemoryStore::open(&db).unwrap();
    assert_eq!(
        store.score(&id_cited).unwrap(),
        1,
        "cited record must score +1"
    );
    assert_eq!(
        store.score(&id_ignored).unwrap(),
        -1,
        "served-but-uncited record must score -1"
    );

    // The deltas are booked on the mission stream (auditable from the log).
    let obs = payloads(&log, stream, EventKind::Observation);
    assert!(
        obs.iter().any(|o| o.contains("\"memory_ledger\"")
            && o.contains(&id_cited)
            && o.contains(&id_ignored)),
        "no memory_ledger booking on the mission stream: {obs:?}"
    );

    let _ = std::fs::remove_dir_all(&dir);
}
