//! UI gap #10 M10 RED: the loop must EMIT model-call UI events.
//!
//! Live-proof finding (tmux, `HS_SEQMODEL_DELAY_MS=350)`: the HUD held
//! "0 calls" for the whole mission and the phase rail never moved -
//! `InnerLoop` emits ToolCallStart/ToolCallEnd but never
//! ModelCallStart/ModelCallEnd, so M5's derivation had nothing to
//! consume until the mission summary landed.

use hs_loop::uipaint::UiEvent;

fn scripted_session(
    dir: &std::path::Path,
) -> (
    hs_loop::repl::ReplSession,
    std::sync::Arc<std::sync::Mutex<Vec<UiEvent>>>,
) {
    let _ = std::fs::remove_dir_all(dir);
    std::fs::create_dir_all(dir).unwrap();
    std::fs::write(
        dir.join("script.jsonl"),
        "Looking at the code now.\n## Done - fixed `parser.rs`, **tests green**\n",
    )
    .unwrap();
    std::fs::write(
        dir.join("hairspring.toml"),
        r#"
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
"#,
    )
    .unwrap();
    unsafe {
        std::env::set_var("HS_SEQMODEL_SCRIPT", dir.join("script.jsonl"));
    }
    let mut s = hs_loop::repl::load_session(
        &dir.join("hairspring.toml"),
        &dir.join("run"),
        false,
        2,
        None,
        None,
    )
    .unwrap();
    let events = std::sync::Arc::new(std::sync::Mutex::new(Vec::<UiEvent>::new()));
    let e2 = events.clone();
    s.set_ui_sink(Box::new(move |ev| e2.lock().unwrap().push(ev)));
    (s, events)
}

// R1: every model call in the mission brackets with Start and End,
// in order, carrying the model name and token counts.
#[test]
fn r1_model_calls_emit_start_and_end() {
    let dir = std::env::temp_dir().join("ui-events-r1");
    let (mut s, events) = scripted_session(&dir);
    let r = s.run_goal("fix the parser").unwrap();
    assert!(r.steps >= 2, "fixture runs 2 steps");
    let evs = events.lock().unwrap();
    let starts = evs
        .iter()
        .filter(|e| matches!(e, UiEvent::ModelCallStart { .. }))
        .count();
    let ends: Vec<&UiEvent> = evs
        .iter()
        .filter(|e| matches!(e, UiEvent::ModelCallEnd { .. }))
        .collect();
    assert!(
        starts >= 2,
        "every model call emits ModelCallStart, got {starts} of >= 2: {evs:?}"
    );
    assert!(
        ends.len() >= 2,
        "every model call emits ModelCallEnd, got {}",
        ends.len()
    );
    for e in &ends {
        match e {
            UiEvent::ModelCallEnd {
                model,
                input_tokens,
                output_tokens,
                cost_usd_micros,
            } => {
                assert!(!model.is_empty(), "ModelCallEnd carries the model name");
                assert!(
                    *input_tokens > 0 && *output_tokens > 0,
                    "ModelCallEnd carries provider token counts"
                );
                assert!(
                    *cost_usd_micros > 0,
                    "ModelCallEnd carries the provider-reported cost (D12)"
                );
            }
            _ => unreachable!(),
        }
    }
    // Start precedes its End.
    let first_start = evs
        .iter()
        .position(|e| matches!(e, UiEvent::ModelCallStart { .. }))
        .unwrap();
    let first_end = evs
        .iter()
        .position(|e| matches!(e, UiEvent::ModelCallEnd { .. }))
        .unwrap();
    assert!(first_start < first_end, "Start brackets End");
}
