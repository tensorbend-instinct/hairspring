//! REPL UI gap #1 (Eric 2026-09-08: "just fix the gaps", SOTA-REPL UI
//! investigation): pi/omp keep an AMBIENT status line on screen at all
//! times - model, mission vitals, running cost - so the operator never
//! types a command to learn where the session stands. hs-repl hides all
//! of it behind `:status` (and even that omits steps/calls).
//!
//! Contract: ReplSession exposes a TYPED vitals snapshot (no string
//! scraping of logs) and the Painter renders it as a one-line ambient
//! status bar - semantic color on a terminal, byte-clean when piped.

use hs_loop::repl::{ReplSession, SessionVitals};
use hs_loop::uipaint::Painter;

const ANSWER: &str = env!("CARGO_BIN_EXE_hs-plugin-answer");
const CHECKER: &str = env!("CARGO_BIN_EXE_hs-plugin-liechecker");
const SCRIPTED: &str = env!("CARGO_BIN_EXE_hs-plugin-scripted");

// HS_SEQMODEL_SCRIPT is process-global: serialize the mission tests.
static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn scripted_config(dir: &std::path::Path) -> std::path::PathBuf {
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

// R1: a fresh session already has a vitals snapshot: the configured
// model's name, zeroed counters, the live stream id.
#[test]
fn r1_vitals_snapshot_of_fresh_session() {
    let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let dir = tempfile::tempdir().unwrap();
    let log = tempfile::tempdir().unwrap();
    let config = scripted_config(dir.path());
    // the scripted plugin reads its script at spawn; no mission runs here
    let script = dir.path().join("idle.jsonl");
    std::fs::write(
        &script,
        serde_json::json!({"tool":"answer.write","args":{"path":"/tmp/never.txt","content":"x"}})
            .to_string(),
    )
    .unwrap();
    unsafe { std::env::set_var("HS_SEQMODEL_SCRIPT", &script) };
    let session = ReplSession::load(&config, log.path(), false, 3).expect("session load");
    let v = session.vitals();
    assert_eq!(v.model_label, "scripted", "vitals name the configured model");
    assert_eq!(v.missions_run, 0);
    assert_eq!(v.total_steps, 0);
    assert_eq!(v.total_model_calls, 0);
    assert_eq!(v.total_cost_micros, 0);
    assert_eq!(v.stream_id, session.stream_id());
}

// R2: vitals ACCUMULATE across missions - after one scripted goal the
// snapshot carries the mission, its steps and its model calls.
#[test]
fn r2_vitals_accumulate_across_missions() {
    let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let dir = tempfile::tempdir().unwrap();
    let log = tempfile::tempdir().unwrap();
    let config = scripted_config(dir.path());
    let answer_path = log
        .path()
        .join("work")
        .join(hs_loop::repl::goal_slug("write the token"))
        .join("answer.txt");
    let script = dir.path().join("s.jsonl");
    std::fs::write(
        &script,
        serde_json::json!({"tool":"answer.write","args":{"path":answer_path,"content":"TOK"}})
            .to_string(),
    )
    .unwrap();
    unsafe { std::env::set_var("HS_SEQMODEL_SCRIPT", &script) };
    let mut session = ReplSession::load(&config, log.path(), false, 3).expect("session load");
    let r = session.run_goal("write the token").expect("goal runs");
    let v = session.vitals();
    assert_eq!(v.missions_run, 1, "one mission ran");
    assert_eq!(v.total_steps, r.steps as u64, "steps accumulate from the mission");
    assert!(
        v.total_model_calls >= 1,
        "model calls accumulate, got {}",
        v.total_model_calls
    );
}

// R3: the Painter renders vitals as a ONE-LINE ambient status bar:
// model, missions, steps, calls, cost in dollars - colored on a
// terminal, ANSI-free when piped.
#[test]
fn r3_painter_status_line() {
    let v = SessionVitals {
        model_label: "deepseek".into(),
        missions_run: 2,
        total_steps: 7,
        total_model_calls: 9,
        total_cost_micros: 430_320,
        elapsed: std::time::Duration::from_secs(65),
        stream_id: uuid::Uuid::parse_str("4c4056c0-ced7-4d5d-9e6a-a85d29a593b9").unwrap(),
    };

    let mut out: Vec<u8> = Vec::new();
    {
        let mut p = Painter::new(&mut out, true /* color */);
        p.status_line(&v);
    }
    let s = String::from_utf8(out).unwrap();
    assert!(s.contains("\x1b["), "colored bar carries ANSI SGR: {s:?}");
    assert_eq!(s.matches('\n').count(), 1, "the bar is one line: {s:?}");
    for needle in ["deepseek", "2 mission", "7 steps", "9 calls", "$0.4303", "4c4056c0"] {
        assert!(s.contains(needle), "bar shows {needle:?}: {s:?}");
    }
    assert!(s.contains('m') && s.contains('s'), "elapsed renders as 1m5s-style: {s:?}");

    let mut out: Vec<u8> = Vec::new();
    {
        let mut p = Painter::new(&mut out, false /* piped */);
        p.status_line(&v);
    }
    let s = String::from_utf8(out).unwrap();
    assert!(!s.contains("\x1b["), "plain bar stays ANSI-free: {s:?}");
    for needle in ["deepseek", "$0.4303", "4c4056c0"] {
        assert!(s.contains(needle), "plain bar shows {needle:?}: {s:?}");
    }
}
