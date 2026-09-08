//! UI gap #10 M12 RED: every mission END must close the goal on the
//! durable stream, so plain sessions are resumable and the delegation
//! graph learns failures.
//!
//! Live-proof finding (2026-09-08, tmux cap5): after two completed
//! scripted missions, `:resume" reported "no prior sessions in this
//! dir to resume". Probe of the real run dir: the operator stream was
//! kinds [0,0,0,0] (ModelCall only) - GoalUpdate is emitted only on
//! the checker-green stop path (lib.rs), so a mission ending exhausted,
//! budget/wall-killed, interrupted, or harness-aborted leaves a stream
//! the picker filter (Feedback|GoalUpdate) is blind to. Plain
//! conversational sessions are the COMMON REPL case.
//!
//! Semantics constraint: hs-swarm's Spawner writes a spawn-time
//! GoalUpdate {done:false} as the child's FIRST event ("open goal").
//! A terminal close is therefore done:true OR an `outcome` key;
//! spawn-time done:false alone stays Running in the graph.

use hs_loop::tui::{self, DelegationGraph};

fn scripted_session(dir: &std::path::Path) -> hs_loop::repl::ReplSession {
    std::fs::create_dir_all(dir).unwrap();
    std::fs::write(
        dir.join("script.jsonl"),
        "Looking at the code now.\n## Done - fixed `parser.rs`, **tests green**\n",
    )
    .unwrap();
    std::fs::write(
        dir.join("hairspring.toml"),
        r#"
[[tools]]
name = "answer.write"
command = ["/mnt/instinct-nvme/hairspring/target/debug/hs-plugin-answer"]
subjects = ["*"]
[[tools]]
name = "checker.run"
command = ["/mnt/instinct-nvme/hairspring/target/debug/hs-plugin-liechecker"]
subjects = ["*"]
[[models]]
name = "scripted"
command = ["/mnt/instinct-nvme/hairspring/target/debug/hs-plugin-scripted"]
default = true
"#,
    )
    .unwrap();
    unsafe {
        std::env::set_var("HS_SEQMODEL_SCRIPT", dir.join("script.jsonl"));
    }
    hs_loop::repl::load_session(&dir.join("hairspring.toml"), &dir.join("run"), false, 2, None, None)
        .unwrap()
}

// R1: a mission that runs to step exhaustion closes the goal on the
// operator stream (terminal GoalUpdate, done:false + outcome), and the
// resume picker then sees the session - operator only, aux excluded.
#[test]
fn r1_exhausted_mission_closes_goal_and_is_resumable() {
    let dir = tempfile::tempdir().unwrap();
    let run = dir.path().join("run");
    let r = {
        let mut s = scripted_session(dir.path());
        s.run_goal("fix the parser").unwrap()
    };
    assert!(!r.passed, "scripted prose never passes the checker");
    assert_eq!(r.outcome, "steps_exhausted");

    let reader = hs_log::StreamReader::open(&run, r.stream_id).unwrap();
    let events = reader.events().unwrap();
    let kinds: Vec<u8> = events.iter().map(|e| e.kind as u8).collect();
    let last = events.last().expect("a mission writes events");
    assert_eq!(
        last.kind,
        hs_core::EventKind::GoalUpdate,
        "the mission's LAST event must close the goal, kinds {kinds:?}"
    );
    let hs_core::Payload::Inline(b) = &last.payload else {
        panic!("GoalUpdate payload is inline json")
    };
    let v: serde_json::Value = serde_json::from_slice(b).unwrap();
    assert_eq!(v["done"], serde_json::json!(false));
    assert_eq!(
        v["outcome"], "steps_exhausted",
        "the close names how the mission ended"
    );

    let infos = hs_loop::repl::list_sessions(&run);
    assert_eq!(
        infos.len(),
        1,
        "operator stream resumable, aux journal excluded: {infos:?}"
    );
    assert_eq!(infos[0].id, r.stream_id);
}

// R2: the delegation graph reads a terminal GoalUpdate done:false as
// FAILED - a child whose mission closed unsuccessfully is not Running
// forever.
#[test]
fn r2_graph_marks_failed_child_from_terminal_goalupdate() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let parent = uuid::Uuid::new_v4();
    let child = uuid::Uuid::new_v4();
    let mut cw = hs_log::StreamWriter::create(root, child).unwrap();
    cw.append(
        hs_core::EventBuilder::new(hs_core::EventKind::GoalUpdate).payload(
            hs_core::Payload::Inline(
                serde_json::to_vec(&serde_json::json!({
                    "mission": "map the parser", "child_of": parent,
                    "done": false, "outcome": "steps_exhausted"
                }))
                .unwrap(),
            ),
        ),
    )
    .unwrap();
    drop(cw);
    let mut pw = hs_log::StreamWriter::create(root, parent).unwrap();
    pw.append(
        hs_core::EventBuilder::new(hs_core::EventKind::Spawn).payload(
            hs_core::Payload::Inline(
                serde_json::to_vec(&serde_json::json!({
                    "child_stream_id": child, "mission": "map the parser"
                }))
                .unwrap(),
            ),
        ),
    )
    .unwrap();
    drop(pw);

    let g = DelegationGraph::scan_stream(root, parent).unwrap();
    assert_eq!(g.nodes().len(), 1);
    assert_eq!(
        g.nodes()[0].status,
        tui::AgentStatus::Failed,
        "a closed goal with done:false is a failed delegation"
    );
}

// R3 (regression): hs-swarm's spawn-time GoalUpdate {done:false}
// carries NO outcome - the goal is OPEN, not failed. The graph must
// keep that child Running.
#[test]
fn r3_spawn_time_open_goal_stays_running() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let parent = uuid::Uuid::new_v4();
    let child = uuid::Uuid::new_v4();
    let mut cw = hs_log::StreamWriter::create(root, child).unwrap();
    cw.append(
        hs_core::EventBuilder::new(hs_core::EventKind::GoalUpdate).payload(
            hs_core::Payload::Inline(
                serde_json::to_vec(&serde_json::json!({
                    "mission": "map the parser", "child_of": parent, "done": false
                }))
                .unwrap(),
            ),
        ),
    )
    .unwrap();
    drop(cw);
    let mut pw = hs_log::StreamWriter::create(root, parent).unwrap();
    pw.append(
        hs_core::EventBuilder::new(hs_core::EventKind::Spawn).payload(
            hs_core::Payload::Inline(
                serde_json::to_vec(&serde_json::json!({
                    "child_stream_id": child, "mission": "map the parser"
                }))
                .unwrap(),
            ),
        ),
    )
    .unwrap();
    drop(pw);

    let g = DelegationGraph::scan_stream(root, parent).unwrap();
    assert_eq!(g.nodes().len(), 1);
    assert_eq!(
        g.nodes()[0].status,
        tui::AgentStatus::Running,
        "spawn-time done:false with no outcome is an OPEN goal"
    );
}
