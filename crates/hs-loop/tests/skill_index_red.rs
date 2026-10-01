//! RED: installed world skills are visible to the model as a compact index
//! in the prompt tail, and skill.list / skill.view tools load them
//! (Hermes-style progressive disclosure).
use hs_core::EventKind;

const ANSWER: &str = env!("CARGO_BIN_EXE_hs-plugin-answer");
const CHECKER: &str = env!("CARGO_BIN_EXE_hs-plugin-checker");
const SCRIPTED: &str = env!("CARGO_BIN_EXE_hs-plugin-scripted");

fn payloads(log: &std::path::Path, stream: uuid::Uuid, kind: EventKind) -> Vec<String> {
    let reader = hs_log::StreamReader::open(log, stream).unwrap();
    reader.events().unwrap().iter().filter(|e| e.kind == kind)
        .filter_map(|e| reader.resolve_payload(e).ok())
        .map(|b| String::from_utf8_lossy(&b).into_owned()).collect()
}

#[test]
fn installed_skill_is_indexed_in_prompt_and_viewable() {
    unsafe { std::env::set_var("HS_SCRIPTED_PROMPT_AWARE", "1") };
    let dir = std::env::temp_dir().join(format!("hsskillidx-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let log = dir.join("log");
    std::fs::create_dir_all(&log).unwrap();

    let world = hs_world::World::open(&log).unwrap();
    let s1 = uuid::Uuid::new_v4();
    hs_log::StreamWriter::create(&log, s1).unwrap();
    let content = "---\nname: bisect-flaky\ndescription: Locate a flaky test's root cause\n---\nSTEP-ONE: rerun 20 times\n";
    use sha2::Digest;
    let a = world.propose(hs_world::Artifact {
        artifact_id: uuid::Uuid::new_v4(), version: 1, kind: hs_world::ArtifactKind::Skill,
        content_hash: sha2::Sha256::digest(content.as_bytes()).into(),
        world_path: "/skills/bisect-flaky".into(), author_stream: s1, parent_version: None,
        status: hs_world::ArtifactStatus::Proposed,
    }, content.as_bytes()).unwrap();
    world.install_skill(a.artifact_id).unwrap();

    let mission = "task-7";
    let answer = log.join("work").join(mission).join("answer.txt");
    let script = dir.join("script.jsonl");
    std::fs::write(&script, [
        serde_json::json!({"tool":"skill.list","args":{}}).to_string(),
        serde_json::json!({"tool":"skill.view","args":{"name":"bisect-flaky"}}).to_string(),
        serde_json::json!({"tool":"answer.write","args":{"path": answer.display().to_string(), "content":"TOKEN-7-SECRET"}}).to_string(),
    ].join("\n")).unwrap();
    let config = dir.join("hairspring.toml");
    std::fs::write(&config, format!(r#"
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
command = ["/bin/sh", "-c", "HS_SCRIPTED_NAME=scripted HS_SEQMODEL_SCRIPT={} exec {SCRIPTED}"]
default = true
subjects = ["*"]
"#, script.display())).unwrap();
    let kernel = hs_kernel::Kernel::load(&config).unwrap();
    let mut l = hs_loop::InnerLoop::new(kernel, &log, true, 10).unwrap();
    l.attach_world();
    let s = l.stream_id();
    let r = l.run_mission(mission).unwrap();
    assert!(r.passed, "{r:?}");

    let calls = payloads(&log, s, EventKind::ToolCall);
    assert!(calls.iter().any(|c| c.contains("skill.list") && c.contains("bisect-flaky")), "skill.list output: {calls:?}");
    assert!(calls.iter().any(|c| c.contains("skill.view") && c.contains("STEP-ONE")), "skill.view output: {calls:?}");
    let mc = payloads(&log, s, EventKind::ModelCall);
    assert!(mc.iter().any(|m| m.contains("bisect-flaky: Locate a flaky test")), "index missing from prompt");
    assert!(mc.iter().all(|m| !m.contains("rerun 20 times") || m.contains("STEP-ONE")), "unreached");
    // The body must only reach the prompt after skill.view, never in the first call.
    assert!(!mc[0].contains("STEP-ONE"), "body leaked into first prompt");
    let _ = std::fs::remove_dir_all(&dir);
}
