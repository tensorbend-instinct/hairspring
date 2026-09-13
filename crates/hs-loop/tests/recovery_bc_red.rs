//! RED-first pin for recovery tiers B and C exercised from the REPL
//! session surface (v5 "Recovery tiers", checklist 7.8):
//!
//! - tier B: the operator snapshots the mission workdir from the session
//!   itself (a `SnapshotRef` lands on the mission stream); after "sandbox
//!   dead" the restore is hash-verified byte-exact (the `hs-world`
//!   primitive) AND books the recovery on the mission stream, so the
//!   `T_mission` R term measures it (B6b);
//! - tier C: VM recycled - recreate from the durable substrate log ALONE:
//!   resume the mission stream, restore its newest snapshot into a fresh
//!   workdir, book the cold recovery measured, and the session resumes.
//!
//! v1: snapshot -> destroy -> restore, evidence on the mission stream.
//! v2: cold, run dir's work tree gone entirely, `cold_recover` rebuilds
//!     it from the log plus the snapshot, then `load_resume` continues.
//! v3: the operators commands parse.

use hs_loop::mission_time::MissionTime;
use hs_loop::repl::{cold_recover, load_session, ReplSession};

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

[[models]]
name = "scripted"
command = ["/mnt/instinct-nvme/hairspring/target/debug/hs-plugin-scripted"]
default = true
subjects = ["*"]
"#;
    std::fs::write(dir.join("hairspring.toml"), toml).unwrap();
    let lines = format!(
        "{{\"tool\":\"answer.submit\",\"args\":{{\"path\":\"{}/work/task-1/answer.txt\",\"content\":\"TOKEN-1-SECRET\"}}}}\n\
         \"nothing further to audit\"\n",
        dir.join("run").display()
    );
    std::fs::write(dir.join("script.jsonl"), lines).unwrap();
}

fn mission_payloads(reader: &hs_log::StreamReader) -> Vec<String> {
    reader
        .events()
        .unwrap()
        .iter()
        .filter_map(|e| reader.resolve_payload(e).ok())
        .map(|b| String::from_utf8_lossy(&b).into_owned())
        .collect()
}

#[test]
fn v1_snapshot_restore_books_recovery_on_the_mission_stream() {
    // FIXME: Audit that the environment access only happens in single-threaded code.
    unsafe { std::env::set_var("HS_ANSWER_RAW", "1") };
    // FIXME: Audit that the environment access only happens in single-threaded code.
    unsafe { std::env::set_var("HS_SCRIPTED_PROMPT_AWARE", "1") };
    let dir = std::env::temp_dir().join("recovery-bc-v1");
    let _ = std::fs::remove_dir_all(&dir);
    write_fixture(&dir);
    // FIXME: Audit that the environment access only happens in single-threaded code.
    unsafe { std::env::set_var("HS_SEQMODEL_SCRIPT", dir.join("script.jsonl")) };
    let run = dir.join("run");
    let mut s = load_session(&dir.join("hairspring.toml"), &run, true, Some(10), None, None).unwrap();
    let r = s.run_goal("task-1").unwrap();
    assert!(r.passed, "mission passes first: {r:?}");
    // state file we own: the recovery property pins the workdir tree,
    // not the scripted plugin's post-pass marker
    let answer = run.join("work/task-1/state.txt");
    std::fs::write(&answer, "FS-STATE-42").unwrap();

    // tier B: snapshot the workdir from the session; the booking lands on
    // the mission stream
    let rep = s.snapshot_workdir().unwrap();
    assert_eq!(rep.snapshot_id.len(), 64, "content-addressed id");
    assert!(rep.files >= 1, "the snapshot captured the answer tree");
    let sid = s.vitals().stream_id;
    let reader = hs_log::StreamReader::open(&run, sid).unwrap();
    let texts = mission_payloads(&reader);
    assert!(
        texts.iter().any(|t| t.contains(&rep.snapshot_id)),
        "the SnapshotRef is on the mission stream: {texts:#?}"
    );

    // sandbox dead: the workdir is destroyed
    std::fs::remove_dir_all(run.join("work")).unwrap();
    assert!(!answer.exists());

    // tier B restore: byte-exact, hash-verified, recovery BOOKED
    let rep2 = s.restore_workdir(&rep.snapshot_id).unwrap();
    assert_eq!(
        std::fs::read_to_string(&answer).unwrap(),
        "FS-STATE-42",
        "byte-exact rehydration ({} files)",
        rep2.files
    );
    let reader = hs_log::StreamReader::open(&run, sid).unwrap();
    let texts = mission_payloads(&reader);
    assert!(
        texts.iter().any(|t| t
            .contains(&format!("recovery tier=B restore snapshot_id={}", rep.snapshot_id))
            && t.contains("duration_ms=")),
        "the restore is booked as recovery on the mission stream: {texts:#?}"
    );

    // and B6b's T_mission decomposition measures it
    let d = MissionTime::decompose(&reader).unwrap();
    assert_eq!(d.r_failures, 1, "the R term counts the recovery");
}

#[test]
fn v2_cold_recreate_from_the_substrate_alone() {
    // FIXME: Audit that the environment access only happens in single-threaded code.
    unsafe { std::env::set_var("HS_ANSWER_RAW", "1") };
    // FIXME: Audit that the environment access only happens in single-threaded code.
    unsafe { std::env::set_var("HS_SCRIPTED_PROMPT_AWARE", "1") };
    let dir = std::env::temp_dir().join("recovery-bc-v2");
    let _ = std::fs::remove_dir_all(&dir);
    write_fixture(&dir);
    // FIXME: Audit that the environment access only happens in single-threaded code.
    unsafe { std::env::set_var("HS_SEQMODEL_SCRIPT", dir.join("script.jsonl")) };
    let run = dir.join("run");
    let mut s = load_session(&dir.join("hairspring.toml"), &run, true, Some(10), None, None).unwrap();
    let r = s.run_goal("task-1").unwrap();
    assert!(r.passed, "mission passes first: {r:?}");
    let answer = run.join("work/task-1/state.txt");
    std::fs::write(&answer, "FS-STATE-42").unwrap();
    let rep = s.snapshot_workdir().unwrap();

    // VM recycled: the entire ephemeral work tree is gone; only the durable
    // substrate (streams + blobs, where snapshots live) survives
    std::fs::remove_dir_all(run.join("work")).unwrap();
    drop(s);

    let rec = cold_recover(&run).unwrap();
    assert_eq!(rec.snapshot_id.as_deref(), Some(rep.snapshot_id.as_str()));
    assert!(rec.files_restored >= 1, "rehydrated from the snapshot");
    assert_eq!(std::fs::read_to_string(&answer).unwrap(), "FS-STATE-42");
    // booked, honestly measured
    let reader = hs_log::StreamReader::open(&run, rec.stream).unwrap();
    let texts = mission_payloads(&reader);
    assert!(
        texts.iter().any(|t| t
            .contains("recovery tier=C cold-recreate")
            && t.contains(&rep.snapshot_id)
            && t.contains("duration_ms=")),
        "cold recovery booked on the mission stream: {texts:#?}"
    );

    // and the session itself resumes on the recreated stream
    let resumed =
        ReplSession::load_resume(&dir.join("hairspring.toml"), &run, true, Some(10), rec.stream)
            .unwrap();
    assert_eq!(resumed.vitals().stream_id, rec.stream);
    drop(resumed);
}

#[test]
fn v3_repl_commands_parse() {
    use hs_loop::repl::{parse_command, ReplCommand};
    assert_eq!(parse_command(":snapshot"), ReplCommand::Snapshot);
    // Cold recreate is a STARTUP operation (recreate from the substrate
    // alone), never a mid-session command: no :recover parse arm exists,
    // because a live session already owns the stream writer.
    assert!(matches!(
        parse_command(":recover"),
        ReplCommand::Unknown(_)
    ));
    assert_eq!(
        parse_command(":restore deadbeef"),
        ReplCommand::Restore("deadbeef".to_string())
    );
}
