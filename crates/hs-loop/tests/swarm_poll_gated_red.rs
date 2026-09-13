//! RED: outcome delivery must fail LOUDLY at spawn time when the
//! config gates `agent.spawn_poll` away from the operator subject.
//! Registration and visibility are different things: a config can
//! register `agent.spawn_poll` with subject list `["child"]` so the
//! registration pre-flight passes, yet every kernel-side poll (sent
//! as subject `operator`) returns `KernelError::Gated`. That error is
//! treated as a transient (the `_ => i += 1` arm in `poll_children`),
//! so the parent books the child, waits on outcomes that can never
//! arrive, and burns every step plus the wall guard. The spawn
//! pre-flight already fails loudly in both cases (subject-filtered
//! `list_tools`); this pins that the GATED arm is diagnosed as such -
//! a refusal text that says "registered" would send the operator
//! hunting a missing binary instead of the mis-scoped config entry.

use hs_loop::repl::load_session;
use std::sync::Mutex;

static SERIAL: Mutex<()> = Mutex::new(());

/// Same fixture family as `swarm_async_red`, but `agent.spawn_poll` is
/// gated away from "operator" - registered, invisible.
fn write_fixture(dir: &std::path::Path) {
    std::fs::create_dir_all(dir).unwrap();
    let toml = r#"
[[tools]]
name = "answer.submit"
command = ["/mnt/instinct-nvme/hairspring/target/debug/hs-plugin-answersubmit"]
subjects = ["*"]

[[tools]]
name = "checker.run"
command = ["/mnt/instinct-nvme/hairspring/target/debug/hs-plugin-liechecker"]
subjects = ["*"]

[[tools]]
name = "agent.spawn"
command = ["/mnt/instinct-nvme/hairspring/target/debug/hs-plugin-swarm"]
subjects = ["*"]

[[tools]]
name = "agent.spawn_poll"
command = ["/mnt/instinct-nvme/hairspring/target/debug/hs-plugin-swarm", "--as", "agent.spawn_poll"]
subjects = ["child"]

[[models]]
name = "scripted-fast"
command = ["/bin/sh", "-c", "HS_SCRIPTED_NAME=scripted-fast HS_SEQMODEL_DELAY_MS=100 exec /mnt/instinct-nvme/hairspring/target/debug/hs-plugin-scripted"]
default = true
subjects = ["*"]
"#;
    std::fs::write(dir.join("hairspring.toml"), toml).unwrap();
    std::fs::write(
        dir.join("script.jsonl"),
        "{\"tool\":\"agent.spawn\",\"args\":{\"mission\":\"c2 probe\"}}\n",
    )
    .unwrap();
}

fn run() -> (hs_loop::MissionResult, String) {
    let dir = std::env::temp_dir().join("swarm-gated-c2");
    let _ = std::fs::remove_dir_all(&dir);
    write_fixture(&dir);
    // FIXME: Audit that the environment access only happens in single-threaded code.
    unsafe { std::env::remove_var("HS_SWARM_DEPTH") };
    unsafe { std::env::set_var("HS_ANSWER_RAW", "1") };
    unsafe { std::env::set_var("HS_SCRIPTED_PROMPT_AWARE", "1") };
    unsafe { std::env::set_var("HS_SEQMODEL_SCRIPT", dir.join("script.jsonl")) };
    unsafe { std::env::set_var("HS_SWARM_LOG_ROOT", dir.join("run")) };
    unsafe { std::env::set_var("HS_SWARM_CONFIG", dir.join("hairspring.toml")) };
    let mut s = load_session(&dir.join("hairspring.toml"), &dir.join("run"), false, Some(12), None, None)
        .unwrap();
    let r = s.run_goal("c2-gated-poll-probe").unwrap();
    let mut all = String::new();
    let reader = hs_log::StreamReader::open(&dir.join("run"), r.stream_id).unwrap();
    for ev in reader.events().unwrap() {
        if let Ok(b) = reader.resolve_payload(&ev) {
            all.push_str(&String::from_utf8_lossy(&b));
        }
    }
    (r, all)
}

#[test]
fn r1_gated_poll_slot_rejects_spawn_with_the_real_cause() {
    let _serial = SERIAL.lock().unwrap();
    let (r, stream) = run();
    assert!(
        stream.contains("agent.spawn_poll is registered but its subjects exclude the operator subject"),
        "spawn must be refused with the visibility cause, stream holds: {}",
        &stream[..stream.len().min(3000)]
    );
    assert!(
        !stream.contains("Spawn "),
        "a refused spawn must never book Spawn provenance"
    );
    // The mission must NOT burn the step budget waiting on
    // outcomes that can never be delivered.
    assert!(
        r.steps <= 4,
        "mission must close fast (spawn refused), got {r:?}"
    );
    assert!(
        r.outcome != "steps_exhausted",
        "gated polls must never be spun on until steps run out: {r:?}"
    );
}
