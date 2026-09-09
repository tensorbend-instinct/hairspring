//! UI gap #10 M14 RED: resuming a session must restore its VISIBLE
//! history. v2 cap6 proved the switch works (HUD follows the resumed
//! stream) but the screen showed only "resumed stream X" - pi/omp
//! restore the session's transcript. The substrate has everything:
//! `GoalUpdate` closes each mission (M12) and `ModelCall` payloads carry
//! messages + completion. `backfill_transcript` replays goal echoes,
//! committed answer blocks, and per-mission done lines through the
//! same render paths as the live surface. Internal distill calls
//! never render.

use hs_loop::tui::{self, TuiState};

fn write_stream(
    root: &std::path::Path,
    id: uuid::Uuid,
    events: Vec<(hs_core::EventKind, serde_json::Value, i64)>,
) {
    let mut w = hs_log::StreamWriter::create(root, id).unwrap();
    for (kind, v, cost) in events {
        w.append(
            hs_core::EventBuilder::new(kind)
                .payload(hs_core::Payload::Inline(serde_json::to_vec(&v).unwrap()))
                .cost_usd_micros(cost),
        )
        .unwrap();
    }
}

fn transcript_text(st: &TuiState) -> String {
    st.transcript
        .iter()
        .map(|l| l.spans.iter().map(|s| s.content.clone()).collect::<String>())
        .collect::<Vec<_>>()
        .join("\n")
}

// R1: a two-mission stream backfills goal echo, answers, and done
// lines in order - distill calls excluded.
#[test]
fn r1_backfill_replays_visible_history() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let id = uuid::Uuid::new_v4();
    write_stream(
        root,
        id,
        vec![
            // mission 1: one operator call, then the terminal GoalUpdate (M12)
            (hs_core::EventKind::ModelCall, serde_json::json!({
                "model": "scripted",
                "messages": [{"role": "user", "content": "MISSION: fix the parser bug\nand keep it green\n\n..."}],
                "completion": "patched the parser, tests green",
                "input_tokens": 50, "output_tokens": 10
            }), 700),
            (hs_core::EventKind::GoalUpdate, serde_json::json!({
                "mission": "fix-the-parser-bug", "done": false, "outcome": "steps_exhausted"
            }), 0),
            // mission 2: operator call + an internal distill call inside
            // the mission window (counted, never rendered)
            (hs_core::EventKind::ModelCall, serde_json::json!({
                "model": "scripted",
                "messages": [{"role": "user", "content": "MISSION: harden the edge cases\n\n..."}],
                "completion": "added the boundary tests",
                "input_tokens": 60, "output_tokens": 12
            }), 600),
            (hs_core::EventKind::ModelCall, serde_json::json!({
                "model": "scripted", "why": "distill",
                "prompt": "distill the transcript", "completion": "INTERNAL NOTES",
                "input_tokens": 40, "output_tokens": 8
            }), 100),
            (hs_core::EventKind::GoalUpdate, serde_json::json!({
                "mission": "harden-the-edge-cases", "done": true, "outcome": "verified"
            }), 0),
        ],
    );

    let mut st = TuiState::default();
    tui::backfill_transcript(&mut st, root, id);
    let text = transcript_text(&st);
    assert!(text.contains("fix the parser bug"), "goal 1 echo: {text:?}");
    assert!(
        text.contains("and keep it green"),
        "multi-line goal survives: {text:?}"
    );
    assert!(text.contains("patched the parser, tests green"), "answer 1: {text:?}");
    assert!(text.contains("harden the edge cases"), "goal 2 echo: {text:?}");
    assert!(text.contains("added the boundary tests"), "answer 2: {text:?}");
    assert!(!text.contains("INTERNAL NOTES"), "distill never renders: {text:?}");
    assert!(text.contains("verified"), "done line names the outcome: {text:?}");
    // cost comes from the recorded per-call cost_usd_micros, never a
    // token-rate estimate: mission 1 = 700 micros, mission 2 = 600+100
    // (distill counts toward cost, never renders).
    assert_eq!(text.matches("$0.0007").count(), 2, "recorded cost, both missions: {text:?}");
    assert!(!text.contains("$0.0003"), "no token-rate estimate: {text:?}");
    // order: goal 1 before answer 1 before goal 2
    let g1 = text.find("fix the parser bug").unwrap();
    let a1 = text.find("patched the parser").unwrap();
    let g2 = text.find("harden the edge cases").unwrap();
    assert!(g1 < a1 && a1 < g2, "stream order preserved: {text:?}");
}

// R2: backfilling a stream with no missions is a no-op, not a panic.
#[test]
fn r2_backfill_empty_stream_is_noop() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let id = uuid::Uuid::new_v4();
    drop(hs_log::StreamWriter::create(root, id).unwrap());
    let mut st = TuiState::default();
    tui::backfill_transcript(&mut st, root, id);
    assert!(st.transcript.is_empty());
    // and a nonexistent stream id: same
    tui::backfill_transcript(&mut st, root, uuid::Uuid::new_v4());
    assert!(st.transcript.is_empty());
}

// R3: the loop records the real per-call cost on the ModelCall
// payload, so a resumed session's done lines can show the SAME cost
// the live HUD showed - not a token-rate estimate (cap11 showed the
// estimate at $0.0007 where live showed $0.0014).
#[test]
fn r3_model_call_payload_records_cost() {
    let dir = std::env::temp_dir().join("m14-r3");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("script.jsonl"), "alpha\nbeta\n").unwrap();
    std::fs::write(dir.join("hairspring.toml"), r#"
[[tools]]
name = "answer.write"
command = ["/mnt/instinct-nvme/hairspring/target/debug/hs-plugin-answer"]
subjects = ["*"]
[[tools]]
name = "checker.run"
command = ["/mnt/instinct-nvme/hairspring/target/debug/hs-plugin-liechecker"]
subjects = ["*"]
[[models]]
name = "scripted"
command = ["/mnt/instinct-nvme/hairspring/target/debug/hs-plugin-scripted"]
default = true
"#).unwrap();
    unsafe { std::env::set_var("HS_SEQMODEL_SCRIPT", dir.join("script.jsonl")); }
    let mut s = hs_loop::repl::load_session(
        &dir.join("hairspring.toml"), &dir.join("run"), false, 2, None, None,
    ).unwrap();
    let r = s.run_goal("probe").unwrap();
    drop(s);
    let reader = hs_log::StreamReader::open(&dir.join("run"), r.stream_id).unwrap();
    let events = reader.events().unwrap();
    let calls: Vec<_> = events.iter().filter(|e| e.kind == hs_core::EventKind::ModelCall).collect();
    assert!(!calls.is_empty());
    for ev in calls {
        assert!(
            ev.cost_usd_micros > 0,
            "ModelCall EVENT records cost_usd_micros: {ev:?}"
        );
    }
}
