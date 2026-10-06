//! A running child honors the control files written by agent.send / agent.interrupt.
use hs_swarm::*;

const ANSWER: &str = env!("CARGO_BIN_EXE_hs-plugin-answer-g5");
const CHECKER: &str = env!("CARGO_BIN_EXE_hs-plugin-checker-g5");
const BENCHMODEL: &str = env!("CARGO_BIN_EXE_hs-plugin-benchmodel-g5");

fn cfg(dir: &std::path::Path) -> std::path::PathBuf {
    let c = dir.join("hairspring.toml");
    std::fs::write(&c, format!("[[tools]]\nname = \"answer.write\"\ncommand = [\"{ANSWER}\"]\nsubjects = [\"*\"]\n\n[[tools]]\nname = \"checker.run\"\ncommand = [\"{CHECKER}\"]\nsubjects = [\"*\"]\n\n[[models]]\nname = \"benchmodel\"\ncommand = [\"{BENCHMODEL}\"]\ndefault = true\n")).unwrap();
    c
}

#[test]
fn c1_interrupt_flag_stops_the_child_before_it_passes() {
    let root = tempfile::tempdir().unwrap();
    let log = root.path().join("log");
    let sp = Spawner::new(&log, &cfg(root.path()), true, 6);
    let parent = uuid::Uuid::new_v4();
    hs_log::StreamWriter::create(&log, parent).unwrap();
    let cid = uuid::Uuid::new_v4();
    let (child, _) = sp.spawn_child(parent, cid, "task-0", None, 1).unwrap();
    std::fs::create_dir_all(log.join("swarm")).unwrap();
    std::fs::write(log.join("swarm").join(format!("{cid}.interrupt")), "").unwrap();
    let r = sp.run_to_completion(&child).unwrap();
    assert!(!r.passed, "interrupted child must not pass: {r:?}");
    assert_eq!(r.steps, 0, "{r:?}");
}

#[test]
fn c2_inbox_message_is_delivered_into_the_child_transcript() {
    let root = tempfile::tempdir().unwrap();
    let log = root.path().join("log");
    let sp = Spawner::new(&log, &cfg(root.path()), true, 6);
    let parent = uuid::Uuid::new_v4();
    hs_log::StreamWriter::create(&log, parent).unwrap();
    let cid = uuid::Uuid::new_v4();
    let (child, _) = sp.spawn_child(parent, cid, "task-1", None, 1).unwrap();
    std::fs::create_dir_all(log.join("swarm")).unwrap();
    let inbox = log.join("swarm").join(format!("{cid}.inbox"));
    std::fs::write(&inbox, "PARENT-SAYS-HELLO\n").unwrap();
    let _ = sp.run_to_completion(&child).unwrap();
    assert!(!inbox.exists(), "inbox message was never drained by the child loop");
}
