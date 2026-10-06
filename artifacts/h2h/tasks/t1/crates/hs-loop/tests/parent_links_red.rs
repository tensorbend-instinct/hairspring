//! Audit: every event in a mission stream except the mission root links
//! to a parent event (no orphan feedback/context/message records).

use hs_loop::repl::load_session;

fn write_fixture(dir: &std::path::Path) {
    std::fs::create_dir_all(dir).unwrap();
    // CARGO_BIN_EXE_* pins the fixture to the binaries cargo just built
    // for THIS profile - a hardcoded /target/debug path silently goes
    // stale the moment the suite only builds --release (the 2026-09-10
    // burn: a pre-summary-contract answersubmit accepted "content" and
    // hid the fixture's contract break).
    let toml = format!(
        r#"
[[tools]]
name = "answer.submit"
command = ["{}"]
subjects = ["*"]

[[tools]]
name = "checker.run"
command = ["{}"]
subjects = ["*"]

[[models]]
name = "scripted"
command = ["{}"]
default = true
context_tokens = 300
subjects = ["*"]
"#,
        env!("CARGO_BIN_EXE_hs-plugin-answersubmit"),
        env!("CARGO_BIN_EXE_hs-plugin-checker"),
        env!("CARGO_BIN_EXE_hs-plugin-scripted")
    );
    std::fs::write(dir.join("hairspring.toml"), toml).unwrap();
    let mut lines = String::new();
    // the current answer.submit contract takes path+summary in raw mode
    // ("content" was only ever swallowed by a stale debug plugin build).
    // The real hs-plugin-checker grades task-1: WRONG-1..6 fail with the
    // expected token named (repairable signal), keeping the mission alive
    // long enough for the 300-token window to cross the pressure
    // threshold (the pre-2026-09-10 fixture leaned on four $error'd
    // submits + fallback rounds for the same window growth; six honest
    // failed rounds reproduce it without the broken contract calls);
    // TOKEN-1-SECRET passes; the verifier (prompt-aware scripted) closes
    // the mission; the trailing prose line is the sacrificial script tail.
    for i in 1..=6 {
        lines.push_str(&format!(
            "{{\"tool\":\"answer.submit\",\"args\":{{\"path\":\"{}/work/task-1/answer.txt\",\"summary\":\"WRONG-{i}\"}}}}\n",
            dir.join("run").display()
        ));
    }
    lines.push_str(&format!(
        "{{\"tool\":\"answer.submit\",\"args\":{{\"path\":\"{}/work/task-1/answer.txt\",\"summary\":\"TOKEN-1-SECRET\"}}}}\n",
        dir.join("run").display()
    ));
    lines.push_str("\"nothing further to audit\"\n");
    std::fs::write(dir.join("script.jsonl"), lines).unwrap();
}


#[test]
fn every_mission_event_links_to_the_mission_root() {
    // FIXME: Audit that the environment access only happens in single-threaded code.
    unsafe { std::env::set_var("HS_ANSWER_RAW", "1") };
    // FIXME: Audit that the environment access only happens in single-threaded code.
    unsafe { std::env::set_var("HS_SCRIPTED_PROMPT_AWARE", "1") };
    let dir = std::env::temp_dir().join("parent-links-red");
    let _ = std::fs::remove_dir_all(&dir);
    write_fixture(&dir);
    // FIXME: Audit that the environment access only happens in single-threaded code.
    unsafe { std::env::set_var("HS_SEQMODEL_SCRIPT", dir.join("script.jsonl")) };
    let run = dir.join("run");
    let mut s = load_session(&dir.join("hairspring.toml"), &run, true, Some(10), None, None).unwrap();
    let r = s.run_goal("task-1").unwrap();
    assert!(r.passed, "mission passes: {r:?}");
    let sid = s.vitals().stream_id;
    let reader = hs_log::StreamReader::open(&run, sid).unwrap();
    let events = reader.events().unwrap();
    let root = events.iter().find(|e| {
        reader.resolve_payload(e).map(|b| String::from_utf8_lossy(&b).contains("mission_start")).unwrap_or(false)
    }).expect("mission_start root exists");
    let orphans: Vec<_> = events.iter().filter(|e| e.event_id != root.event_id && e.seq > root.seq && e.parent_event_id.is_none()).map(|e| (e.seq, format!("{:?}", e.kind))).collect();
    assert!(orphans.is_empty(), "events after mission_start without a parent link: {orphans:?}");
}
