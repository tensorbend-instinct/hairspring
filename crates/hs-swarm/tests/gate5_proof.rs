//! GATE 5 ACCEPTANCE (spec section 10, row 5):
//!   A delegated subtask runs the same substrate as a child stream;
//!   delegation overhead measured in milliseconds, not deployment.
//!
//! Falsifiable: child streams missing/unverifiable in the parent's log
//! root, spawn events not naming child `stream_ids`, or overhead not measured
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
    // Single-authority fencing (spec 2.6): the parent stream exists but
    // holds NO live writer while the spawner books the delegation - in
    // production the parent loop owns its stream and books Spawn itself
    // at mint time; this crate helper stands in for that moment.
    hs_log::StreamWriter::create(&log_root, parent_stream).unwrap();

    let spawner = Spawner::new(&log_root, &config, true, 6);
    let mut overheads = Vec::new();
    let mut children = Vec::new();
    for c in 0..CHILDREN {
        let (child, overhead_ms) = spawner
            .spawn(&log_root, parent_stream, &format!("task-{c}"))
            .unwrap();
        overheads.push(overhead_ms);
        children.push(child);
    }

    // every child runs to completion on the same substrate
    let reports: Vec<ChildReport> = children
        .iter()
        .map(|c| spawner.run_to_completion(c).unwrap())
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
    let reader = hs_log::StreamReader::open(log_root, stream).unwrap();
    reader
        .events()
        .unwrap()
        .iter()
        .map(|e| {
            let payload = reader.resolve_payload(e).unwrap();
            serde_json::json!({
                "kind": format!("{:?}", e.kind).to_lowercase(),
                "payload": serde_json::from_slice::<serde_json::Value>(&payload).unwrap(),
            })
        })
        .collect()
}

#[test]
fn gate5_adversarial_failed_child_is_recorded_honestly() {
    let root = tempfile::tempdir().unwrap();
    let log_root = root.path().join("log");
    let config = kernel_config(root.path());
    let parent_stream = uuid::Uuid::new_v4();
    // fenced (spec 2.6): no live parent writer while spawn books Spawn
    hs_log::StreamWriter::create(&log_root, parent_stream).unwrap();

    let spawner = Spawner::new(&log_root, &config, true, 4);
    // two good subtasks, one that can never pass (checker has no task-20)
    let (c0, _) = spawner.spawn(&log_root, parent_stream, "task-0").unwrap();
    let (c1, _) = spawner.spawn(&log_root, parent_stream, "task-1").unwrap();
    let (c2, _) = spawner.spawn(&log_root, parent_stream, "task-20").unwrap();

    let r0 = spawner.run_to_completion(&c0).unwrap();
    let r2 = spawner.run_to_completion(&c2).unwrap(); // fails, but returns a report
    let r1 = spawner.run_to_completion(&c1).unwrap();

    assert!(r0.passed && r1.passed, "good children pass");
    assert!(!r2.passed, "unpassable child reports failure, not a crash");
    for r in [&r0, &r1, &r2] {
        hs_log::verify_stream(&log_root, r.stream_id).unwrap();
    }
    println!("PROOF-GATE5 adversarial: failed child returned passed=false, all chains verify");
}
