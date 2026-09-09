//! RED-first (B2, v5 gate 6 + spec 3.4): the shared world is reachable
//! from missions as native tools. Agents write proposals; the world
//! service alone validates and writes consequences.
//!
//! w1 zero-message coordination: mission B reuses mission A's artifact by
//!    world observation, zero messages (`SwarmWorld` assay shape). The
//!    world stream carries Proposal THEN Consequence - the separation.
//! w2 validation: illegal proposals are rejected with the reason served
//!    to the model; proposal booked, consequence ABSENT.
//! w3 executable inheritance: an installed controller keeps acting on
//!    world ticks after its author stream is uninstalled.
//! w4 unattached world plane -> explicit, non-fatal tool error.

use hs_core::EventKind;
use hs_loop::*;

static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

const ANSWER: &str = env!("CARGO_BIN_EXE_hs-plugin-answer");
const CHECKER: &str = env!("CARGO_BIN_EXE_hs-plugin-checker");
const SCRIPTED: &str = env!("CARGO_BIN_EXE_hs-plugin-scripted");

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

fn answer_line(log: &std::path::Path, task: &str) -> serde_json::Value {
    serde_json::json!({
        "tool":"answer.write",
        "args":{"path":format!("{}/work/{}/answer.txt", log.display(), task),
                "content":format!("{}-SECRET", task.replace("task-", "TOKEN-"))}
    })
}

fn tool_payloads(log: &std::path::Path, stream: uuid::Uuid) -> Vec<String> {
    let reader = hs_log::StreamReader::open(log, stream).unwrap();
    reader
        .events()
        .unwrap()
        .iter()
        .filter(|e| e.kind == EventKind::ToolCall)
        .filter_map(|e| reader.resolve_payload(e).ok())
        .map(|b| String::from_utf8_lossy(&b).into_owned())
        .collect()
}

fn world_events(log: &std::path::Path, stream: uuid::Uuid) -> Vec<(EventKind, String)> {
    let reader = hs_log::StreamReader::open(log, stream).unwrap();
    reader
        .events()
        .unwrap()
        .iter()
        .filter(|e| e.kind == EventKind::Proposal || e.kind == EventKind::Consequence)
        .map(|e| {
            (
                e.kind,
                String::from_utf8_lossy(&reader.resolve_payload(e).unwrap()).into_owned(),
            )
        })
        .collect()
}

#[test]
fn w1_mission_b_observes_mission_a_artifact_zero_messages() {
    let _guard = ENV_LOCK.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let dir = tempfile::tempdir().unwrap();
    let log = tempfile::tempdir().unwrap();
    let script = write_script(
        dir.path(),
        &[
            serde_json::json!({"tool":"world.propose","args":{"world_path":"/missions/w1/answer","content":"W1-COORDARTIFACT-9Z","kind":"file"}}),
            answer_line(log.path(), "task-4"),
            // the adversarial verifier audit consumes one model call
            // after each green checker; feed it a sacrificial prose line
            serde_json::json!("nothing further to audit"),
            serde_json::json!({"tool":"world.observe","args":{"world_path":"/missions/w1/answer"}}),
            answer_line(log.path(), "task-5"),
        ],
    );
    unsafe { std::env::set_var("HS_SEQMODEL_SCRIPT", &script) };
    let mut l = rig(dir.path(), log.path(), 6);
    l.attach_world();
    let stream = l.stream_id();
    let r1 = l.run_mission("task-4").unwrap();
    assert!(r1.passed, "mission A passes: {r1:?}");
    let r2 = l.run_mission("task-5").unwrap();
    assert!(r2.passed, "mission B passes: {r2:?}");

    let calls = tool_payloads(log.path(), stream);
    let propose = calls
        .iter()
        .find(|p| p.contains("\"plugin\":\"world.propose\""))
        .expect("mission A proposed to the world");
    let observe = calls
        .iter()
        .find(|p| p.contains("\"plugin\":\"world.observe\""))
        .unwrap_or_else(|| {
            eprintln!("CALLS-DUMP: {calls:#?}");
            panic!("mission B observed the world")
        });
    // the proposal carries the agent's own stream as author
    assert!(
        propose.contains(&stream.to_string()),
        "proposal authored by the mission's stream: {propose}"
    );
    // and mission B is served the artifact mission A created, no messages
    assert!(
        observe.contains("artifact_id"),
        "world observation serves artifacts: {observe}"
    );
    assert!(
        observe.contains("\"version\":1"),
        "mission A's artifact served: {observe}"
    );
    // the proposal-consequence separation is booked on the world stream
    let world = world_events(log.path(), l.world_stream_id());
    assert!(
        world.len() >= 2,
        "world stream has proposal + consequence: {world:?}"
    );
    assert_eq!(world[0].0, EventKind::Proposal, "agents write proposals");
    assert_eq!(world[1].0, EventKind::Consequence, "the world validates");
    assert!(
        world[1].1.contains("\"validated\""),
        "consequence is the validated artifact: {}",
        world[1].1
    );
}

#[test]
fn w2_world_rejects_illegal_proposals_with_the_reason_served() {
    let _guard = ENV_LOCK.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let dir = tempfile::tempdir().unwrap();
    let log = tempfile::tempdir().unwrap();
    let script = write_script(
        dir.path(),
        &[
            serde_json::json!({"tool":"world.propose","args":{"world_path":"missions/relative","content":"X","kind":"file"}}),
            answer_line(log.path(), "task-6"),
        ],
    );
    unsafe { std::env::set_var("HS_SEQMODEL_SCRIPT", &script) };
    let mut l = rig(dir.path(), log.path(), 4);
    l.attach_world();
    let stream = l.stream_id();
    let r = l.run_mission("task-6").unwrap();
    assert!(r.passed, "mission passes despite the rejection: {r:?}");

    let calls = tool_payloads(log.path(), stream);
    let propose = calls
        .iter()
        .find(|p| p.contains("\"plugin\":\"world.propose\""))
        .expect("the proposal call was dispatched");
    assert!(
        propose.contains("error"),
        "the rejection is served to the model, not fatal: {propose}"
    );
    assert!(
        propose.contains("absolute"),
        "the reason is named: {propose}"
    );
    // the world stream books the attempt but never a consequence for it
    let world = world_events(log.path(), l.world_stream_id());
    assert_eq!(
        world
            .iter()
            .filter(|(k, _)| *k == EventKind::Consequence)
            .count(),
        0,
        "no consequence for a rejected proposal: {world:?}"
    );
}

#[test]
fn w3_installed_controller_survives_author_uninstall() {
    let _guard = ENV_LOCK.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let dir = tempfile::tempdir().unwrap();
    let log = tempfile::tempdir().unwrap();
    let program = r#"{"op":"append_counter","target":"/missions/w3/counter"}"#;
    let script = write_script(
        dir.path(),
        &[
            serde_json::json!({"tool":"world.propose","args":{"world_path":"/missions/w3/controller","content":program,"kind":"controller"}}),
            answer_line(log.path(), "task-7"),
        ],
    );
    unsafe { std::env::set_var("HS_SEQMODEL_SCRIPT", &script) };
    let mut l = rig(dir.path(), log.path(), 4);
    l.attach_world();
    let stream = l.stream_id();
    let r = l.run_mission("task-7").unwrap();
    assert!(r.passed, "mission A passes: {r:?}");

    // install via the world plane, tick once: the controller acts with no
    // model call (spec: runs_without_model_call)
    let world = l.world().expect("the session owns a world");
    let artifact_id = world
        .observe("/missions/w3/controller")
        .expect("observe")
        .pop()
        .expect("the proposed controller")
        .artifact_id;
    world.install(artifact_id).expect("install");
    world.tick().expect("first tick");
    let n1 = world.observe("/missions/w3/counter").expect("observe").len();
    assert_eq!(n1, 1, "controller acted once");

    // the gate-6 bar: uninstall the author entirely; the controller keeps
    // acting (executable inheritance, SwarmWorld's assay shape)
    world.uninstall_agent(stream).expect("uninstall author");
    world.tick().expect("tick after uninstall");
    let n2 = world.observe("/missions/w3/counter").expect("observe").len();
    assert_eq!(
        n2, 2,
        "the installed controller is world property: {n1} -> {n2}"
    );
}

#[test]
fn w4_world_tools_unattached_are_a_clean_error() {
    let _guard = ENV_LOCK.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let dir = tempfile::tempdir().unwrap();
    let log = tempfile::tempdir().unwrap();
    let script = write_script(
        dir.path(),
        &[
            serde_json::json!({"tool":"world.observe","args":{"world_path":"/missions/w1/answer"}}),
            answer_line(log.path(), "task-8"),
        ],
    );
    unsafe { std::env::set_var("HS_SEQMODEL_SCRIPT", &script) };
    let mut l = rig(dir.path(), log.path(), 4);
    let stream = l.stream_id();
    let r = l.run_mission("task-8").unwrap();
    assert!(r.passed, "mission passes despite the error: {r:?}");
    let calls = tool_payloads(log.path(), stream);
    let call = calls
        .iter()
        .find(|p| p.contains("\"plugin\":\"world.observe\""))
        .expect("dispatched");
    assert!(call.contains("no world attached"), "explicit error: {call}");
}
