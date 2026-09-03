//! GATE 5 ACCEPTANCE (spec section 10, row 5):
//!   A delegated subtask runs the same substrate as a child stream;
//!   delegation overhead measured in milliseconds, not deployment.
//!
//! Falsifiable: child streams missing/unverifiable in the parent's log
//! root, spawn events not naming child stream_ids, or overhead not measured
//! and published in ms all fail the gate.

use hs_swarm::*;

const ANSWER: &str = env!("CARGO_BIN_EXE_hs-plugin-answer-g5");
const CHECKER: &str = env!("CARGO_BIN_EXE_hs-plugin-checker-g5");
const BENCHMODEL: &str = env!("CARGO_BIN_EXE_hs-plugin-benchmodel-g5");
const CHILDREN: usize = 8;

fn kernel_config(dir: &std::path::Path) -> std::path::PathBuf {
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
name = "benchmodel"
command = ["{BENCHMODEL}"]
default = true
"#
        ),
    )
    .unwrap();
    config
}

#[test]
fn gate5_proof_child_streams_and_delegation_overhead() {
    let root = tempfile::tempdir().unwrap();
    let log_root = root.path().join("log");
    let config = kernel_config(root.path());

    // parent stream exists first
    let parent_stream = uuid::Uuid::new_v4();
    let _parent_writer = hs_log::StreamWriter::create(&log_root, parent_stream).unwrap();

    let spawner = Spawner::new(&log_root, &config, true, 6);
    let mut overheads = Vec::new();
    let mut children = Vec::new();
    for c in 0..CHILDREN {
        let (child, overhead_ms) = spawner.spawn(&log_root, parent_stream, &format!("task-{c}"));
        overheads.push(overhead_ms);
        children.push(child);
    }

    // every child runs to completion on the same substrate
    let reports: Vec<ChildReport> = children
        .iter()
        .map(|c| spawner.run_to_completion(c))
        .collect();

    // THE GATE:
    // 1. all child chains verify inside the SAME log root
    for r in &reports {
        hs_log::verify_stream(&log_root, r.stream_id).unwrap();
    }
    // 2. parent stream contains a Spawn event per child naming its stream_id
    let parent_events = read_all_payloads(&log_root, parent_stream);
    let spawn_payloads: Vec<&serde_json::Value> = parent_events
        .iter()
        .filter(|e| e["kind"] == "spawn")
        .collect();
    assert_eq!(spawn_payloads.len(), CHILDREN, "one spawn event per child");
    for r in &reports {
        assert!(
            spawn_payloads
                .iter()
                .any(|e| e["payload"]["child_stream_id"] == r.stream_id.to_string()),
            "no spawn event names child {}",
            r.stream_id
        );
    }
    // 3. children completed their subtasks
    assert!(
        reports.iter().all(|r| r.passed),
        "every delegated subtask passes"
    );
    // 4. delegation overhead measured in ms, median published
    overheads.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let median = overheads[CHILDREN / 2];
    assert!(
        median < 1000.0,
        "delegation overhead {median}ms is deployment, not a spawn call"
    );
    println!("PROOF-GATE5 child streams + delegation overhead: PASS");
    println!("  {CHILDREN} child streams verified in the parent's log root; spawn events name every child");
    println!(
        "  delegation overhead ms: min {:.1} median {:.1} max {:.1}",
        overheads[0],
        median,
        overheads[CHILDREN - 1]
    );
}

fn read_all_payloads(log_root: &std::path::Path, stream: uuid::Uuid) -> Vec<serde_json::Value> {
    let _ = (log_root, stream);
    unimplemented!("gate 5 red: stream payload reader")
}
