//! ACCOUNTING RESET AFTER RESUME (Eric 2026-09-10: "Did you fix the
//! accounting reset issue properly?"). Live symptom, reproduced on the
//! box: quit hs-repl, relaunch, :resume the session - the transcript
//! replays from the log, but every accounting surface reads ZERO
//! (HUD totals, :status cost_usd_micros, the session vitals behind the
//! panels). Root cause: `InnerLoop::with_stream` and
//! `ReplSession::load_resume` adopt the stream but initialize every
//! counter at 0. Contract: a resumed session's accounting equals what
//! the original session reported at quit - the log is the source of
//! truth, resume folds it back. This also closes a budget hole: pre-fix
//! a restart silently reset the session budget guard along with the
//! display.

use hs_core::{EventBuilder, EventKind, Payload};
use hs_loop::repl::ReplSession;

static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

const ANSWER: &str = env!("CARGO_BIN_EXE_hs-plugin-answer");
const CHECKER: &str = env!("CARGO_BIN_EXE_hs-plugin-checker");
const SCRIPTED: &str = env!("CARGO_BIN_EXE_hs-plugin-scripted");

fn write_config(dir: &std::path::Path) -> std::path::PathBuf {
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
    config
}

#[test]
fn resume_restores_session_accounting() {
    let _guard = ENV_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let dir = tempfile::tempdir().unwrap();
    let log = tempfile::tempdir().unwrap();
    let config = write_config(dir.path());

    // session 1: one scripted mission; its own vitals define parity
    let answer1 = log.path().join("work").join("task-0").join("answer.txt");
    let script1 = dir.path().join("s1.jsonl");
    std::fs::write(
        &script1,
        serde_json::json!({"tool":"answer.write","args":{"path":answer1.display().to_string(),"content":"MARKER-ACCT-6613"}})
            .to_string(),
    )
    .unwrap();
    unsafe { std::env::set_var("HS_SEQMODEL_SCRIPT", &script1) };
    let (stream_id, before) = {
        let mut s1 = ReplSession::load(&config, log.path(), true, Some(1)).unwrap();
        s1.run_goal("task-0").unwrap();
        let v = s1.vitals();
        assert_eq!(v.missions_run, 1, "one mission ran: {v:?}");
        assert!(v.total_steps >= 1, "steps were booked: {v:?}");
        assert!(v.total_model_calls >= 1, "calls were booked: {v:?}");
        (s1.stream_id(), v)
    };

    // the log is the source of truth: a priced ModelCall on the stream
    // must land in the resumed session's totals (scripted model reports
    // zero cost, so this is the only way to exercise the cost fold
    // without network)
    {
        let mut w = hs_log::StreamWriter::resume(log.path(), stream_id)
            .unwrap()
            .writer;
        w.append(
            EventBuilder::new(EventKind::ModelCall).payload(Payload::Inline(
                serde_json::to_vec(&serde_json::json!({
                    "model": "scripted", "completion": "{}",
                    "cost_usd_micros": 7000, "conservative_cost_usd_micros": 9000
                }))
                .unwrap(),
            )),
        )
        .unwrap();
        // a verifier-round call: booked as a call and as spend, but NOT
        // as a loop step (mirrors the live loop's bookkeeping; its
        // durable event gained the cost fields in the same fix)
        w.append(
            EventBuilder::new(EventKind::ModelCall).payload(Payload::Inline(
                serde_json::to_vec(&serde_json::json!({
                    "role": "verifier", "round": 1, "model": "scripted",
                    "completion": "{\"tool\":\"verdict.submit\"}",
                    "cost_usd_micros": 11000, "conservative_cost_usd_micros": 13000
                }))
                .unwrap(),
            )),
        )
        .unwrap();
    }

    // session 2 (the restart): resume - accounting must survive
    let s2 = ReplSession::load_resume(&config, log.path(), true, Some(1), stream_id).unwrap();
    let after = s2.vitals();
    assert_eq!(
        after.missions_run, before.missions_run,
        "missions_run resets on resume"
    );
    assert_eq!(
        after.total_steps,
        before.total_steps + 1,
        "steps reset on resume (incl. the priced synthetic call)"
    );
    assert_eq!(
        after.total_model_calls,
        before.total_model_calls + 2,
        "model calls reset on resume (plain + verifier synthetics)"
    );
    assert_eq!(
        after.total_cost_micros,
        before.total_cost_micros + 7000 + 11000,
        "cost resets on resume"
    );
    assert_eq!(
        after.conservative_cost_micros,
        before.conservative_cost_micros + 9000 + 13000,
        "conservative cost resets on resume"
    );
}
