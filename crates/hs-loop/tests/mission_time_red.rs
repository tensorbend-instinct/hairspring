//! RED-first pin for the `T_mission` decomposition (v5 spec: "The
//! mission-time model", checklist 7.9). Mission time decomposes into terms
//! the system measures from its own event log:
//!
//!   `T = N_steps * (t_model + t_overhead) + R_failures * t_recover
//!        + C_coord + S_stuck`
//!
//! v1: a synthetic stream with hand-known latencies decomposes EXACTLY.
//! v2: a real scripted mission decomposes from its own substrate log and
//!     the decomposition report is published as an artifact naming every
//!     term, win or lose.

use hs_core::{EventKind, Payload};
use hs_loop::mission_time::MissionTime;
use hs_loop::repl::load_session;

fn write_mission_fixture(dir: &std::path::Path) {
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
    let mut lines = String::new();
    // one wrong submission (a ToolCall repeat risk), then the pass, then the
    // verifier sacrificial final line so the verifier has a call left.
    lines.push_str(&format!(
        "{{\"tool\":\"answer.submit\",\"args\":{{\"path\":\"{}/work/task-1/answer.txt\",\"content\":\"WRONG\"}}}}\n",
        dir.join("run").display()
    ));
    lines.push_str(&format!(
        "{{\"tool\":\"answer.submit\",\"args\":{{\"path\":\"{}/work/task-1/answer.txt\",\"content\":\"TOKEN-1-SECRET\"}}}}\n",
        dir.join("run").display()
    ));
    lines.push_str("\"nothing further to audit\"\n");
    std::fs::write(dir.join("script.jsonl"), lines).unwrap();
}

#[test]
fn v1_synthetic_log_decomposes_exactly() {
    let dir = std::env::temp_dir().join("mission-time-v1");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let sid = uuid::Uuid::nil();
    let mut w = hs_log::StreamWriter::create(&dir, sid).unwrap();

    let mc = |lat: u32, asm: u64| {
        hs_core::EventBuilder::new(EventKind::ModelCall)
            .payload(Payload::Inline(
                serde_json::to_vec(&serde_json::json!({"assembly_ms": asm})).unwrap(),
            ))
            .latency_ms(lat)
    };
    let tc = || {
        hs_core::EventBuilder::new(EventKind::ToolCall).payload(Payload::Inline(
            serde_json::to_vec(&serde_json::json!({
                "plugin": "sh", "args": {"cmd": "check.sh"},
            }))
            .unwrap(),
        ))
    };
    // MC1(900,40), TC, Message, MC2(1100,60), TC, TC  -> the signature
    // appears 3 times: occurrences 2 and 3 are stuck repeats, each priced
    // at the model step that produced it (step price = latency + assembly).
    w.append(mc(900, 40)).unwrap();
    w.append(tc()).unwrap();
    w.append(
        hs_core::EventBuilder::new(EventKind::Message)
            .payload(Payload::Inline(b"{}".to_vec())),
    )
    .unwrap();
    w.append(mc(1100, 60)).unwrap();
    w.append(tc()).unwrap();
    w.append(tc()).unwrap();

    let reader = hs_log::StreamReader::open(&dir, sid).unwrap();
    let d = MissionTime::decompose(&reader).unwrap();

    assert_eq!(d.n_steps, 2, "two model calls = two steps");
    assert_eq!(d.t_model_ms, 2000, "model latency sums");
    assert_eq!(d.t_overhead_ms, 100, "assembly sums");
    assert_eq!(d.hot_path_ms(), 2100, "N_steps * (t_model + t_overhead)");
    assert_eq!(d.r_failures, 0, "no recovery booked on this stream");
    assert_eq!(d.t_recover_ms, 0);
    assert_eq!(d.c_coord_events, 1, "the message is coordination");
    assert_eq!(d.s_stuck_repeats, 2, "second and third identical call");
    assert_eq!(
        d.s_stuck_ms,
        2 * (1100 + 60),
        "each repeat priced at its producing model step"
    );
    assert_eq!(d.terms_ms(), 2100 + 2 * 1160, "decomposition terms sum");
    assert_eq!(
        i64::try_from(d.terms_ms()).unwrap() + d.unattributed_ms,
        d.wall_ms,
        "terms + unattributed = measured wall"
    );
}

#[test]
fn v2_live_mission_decomposes_and_publishes() {
    // FIXME: Audit that the environment access only happens in single-threaded code.
    unsafe { std::env::set_var("HS_ANSWER_RAW", "1") };
    // FIXME: Audit that the environment access only happens in single-threaded code.
    unsafe { std::env::set_var("HS_SCRIPTED_PROMPT_AWARE", "1") };
    let dir = std::env::temp_dir().join("mission-time-v2");
    let _ = std::fs::remove_dir_all(&dir);
    write_mission_fixture(&dir);
    // FIXME: Audit that the environment access only happens in single-threaded code.
    unsafe { std::env::set_var("HS_SEQMODEL_SCRIPT", dir.join("script.jsonl")) };
    let run = dir.join("run");
    let mut s = load_session(&dir.join("hairspring.toml"), &run, true, Some(10), None, None).unwrap();
    let r = s.run_goal("task-1").unwrap();
    assert!(r.passed, "mission passes: {r:?}");

    let sid = s.vitals().stream_id;
    let reader = hs_log::StreamReader::open(&run, sid).unwrap();
    let events = reader.events().unwrap();
    let d = MissionTime::decompose(&reader).unwrap();

    // cross-check every term against an independent recomputation
    let mcs: Vec<_> = events
        .iter()
        .filter(|e| e.kind == EventKind::ModelCall)
        .collect();
    assert_eq!(d.n_steps, mcs.len() as u64);
    let expect_model: u64 = mcs.iter().map(|e| u64::from(e.latency_ms)).sum();
    assert_eq!(d.t_model_ms, expect_model);
    let expect_asm: u64 = mcs
        .iter()
        .filter_map(|e| reader.resolve_payload(e).ok())
        .filter_map(|b| serde_json::from_slice::<serde_json::Value>(&b).ok())
        .filter_map(|v| v.get("assembly_ms").and_then(serde_json::Value::as_u64))
        .sum();
    assert_eq!(d.t_overhead_ms, expect_asm);
    assert_eq!(
        i64::try_from(d.terms_ms()).unwrap() + d.unattributed_ms,
        d.wall_ms
    );

    // the decomposition is PUBLISHED as an artifact, every term named
    let out_dir = run.join("reports");
    let path = d.write_report(&out_dir, "task-1").unwrap();
    let body = std::fs::read_to_string(&path).unwrap();
    for needle in [
        "T_mission",
        "N_steps=",
        "t_model_total_ms=",
        "t_overhead_total_ms=",
        "hot_path_ms=",
        "R_failures=",
        "t_recover_total_ms=",
        "C_coord",
        "S_stuck",
        "terms_total_ms=",
        "wall_ms=",
        "unattributed_ms=",
        "win or lose",
    ] {
        assert!(body.contains(needle), "report names {needle}: {body}");
    }
}
