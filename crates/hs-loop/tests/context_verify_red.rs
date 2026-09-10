//! B5 verify (v5 spec 3.5/3.6/6.2, checklist items 3.5/3.6/6.2):
//! window mutation is never silent - every injection path books
//! `context_inject` WITH ITS WHY, pressure compaction names the compacted
//! source event range, and the compacted artifact links back to the
//! source range (spec 3.5: "compaction summaries that link back to
//! source event ranges"; 3.6/6.2: record `context_inject`, why=pressure).
//!
//! v1: a mission under a deliberately tiny context budget compacts
//!     mid-run; the stream then contains (a) a `ContextInject` event
//!     "why=pressure compacted=N range=seqLO..seqHI", (b) the BOOKED
//!     distill call itself (cost/latency on the record), (c) the
//!     model-visible COMPACTED pointer embedding the seq+ref range
//!     (the link-back), and (d) EVERY `ContextInject` payload on the
//!     stream carries a why - no unexplained window mutation exists.

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
fn v1_pressure_compaction_is_booked_with_why_and_source_range() {
    // FIXME: Audit that the environment access only happens in single-threaded code.
    unsafe { std::env::set_var("HS_ANSWER_RAW", "1") };
    // FIXME: Audit that the environment access only happens in single-threaded code.
    unsafe { std::env::set_var("HS_SCRIPTED_PROMPT_AWARE", "1") };
    let dir = std::env::temp_dir().join("context-verify-v1");
    let _ = std::fs::remove_dir_all(&dir);
    write_fixture(&dir);
    // FIXME: Audit that the environment access only happens in single-threaded code.
    unsafe { std::env::set_var("HS_SEQMODEL_SCRIPT", dir.join("script.jsonl")) };
    let run = dir.join("run");
    let mut s = load_session(&dir.join("hairspring.toml"), &run, true, 10, None, None).unwrap();
    let r = s.run_goal("task-1").unwrap();
    assert!(r.passed, "mission passes under pressure: {r:?}");

    let sid = s.vitals().stream_id;
    let reader = hs_log::StreamReader::open(&run, sid).unwrap();
    let mut injects: Vec<String> = vec![];
    let mut model_calls: Vec<String> = vec![];
    for ev in reader.events().unwrap() {
        if let Ok(b) = reader.resolve_payload(&ev) {
            let text = String::from_utf8_lossy(&b).to_string();
            match ev.kind {
                hs_core::EventKind::ContextInject => injects.push(text),
                hs_core::EventKind::ModelCall => model_calls.push(text),
                _ => {}
            }
        }
    }

    // (d) every window mutation states its why
    assert!(!injects.is_empty(), "the mission injected context at all");
    for p in &injects {
        assert!(p.contains("why"), "every context_inject carries why: {p}");
    }
    // (a) the pressure booking names the compacted source range
    let pressure = injects
        .iter()
        .find(|p| p.contains("why=pressure compacted=") && p.contains("range=seq"))
        .unwrap_or_else(|| {
            panic!("pressure compaction booked with why + source range; injects: {injects:#?}")
        });
    let range = pressure
        .split("range=seq")
        .nth(1)
        .expect("range present")
        .split_whitespace()
        .next()
        .expect("range value");
    let (lo, hi) = range.split_once("..seq").expect("LO..HI shape");
    assert!(
        lo.parse::<u64>().is_ok() && hi.parse::<u64>().is_ok(),
        "the range pins real event seqs: {range}"
    );
    // (b) the distill call itself is a booked model call
    assert!(
        model_calls.iter().any(|p| p.contains("distill")),
        "the distill call is on the record with its cost"
    );
    // (c) the model-visible COMPACTED pointer embeds the same link-back
    assert!(
        model_calls
            .iter()
            .any(|p| p.contains("COMPACTED ") && p.contains("events seq ") && p.contains("refs ")),
        "a later model call sees the COMPACTED pointer with the seq+ref range"
    );
}
