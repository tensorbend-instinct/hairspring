//! REPL UI gap #10 M18: HUD under-counts model calls on a
//! tool-call-then-answer mission (found in the M17 live proof,
//! /tmp/tui-proof-m17/capC2-done.ansi: HUD "1 mission . 1 steps . 1
//! calls" vs done-line "1 steps, 2 calls", SAME screen; the $0.0014
//! cost = 2 x $0.0007/call proves two real calls, so the HUD live
//! counter dropped exactly one `ModelCallEnd`).
//!
//! The M10 contract pinned starts>=2/ends>=2 on a TEXT-ONLY script.
//! The missing shape was [tool call, final answer]: one of those two
//! calls' End never reaches the UI sink.
//!
//! Contract: the UI event stream accounts for EVERY model call the
//! `MissionResult` counts - exactly `model_calls` Starts and exactly
//! `model_calls` Ends, in any mission shape.

use hs_loop::uipaint::UiEvent;

static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn kind(e: &UiEvent) -> &'static str {
    match e {
        UiEvent::ModelCallStart { .. } => "ModelCallStart",
        UiEvent::ModelCallEnd { .. } => "ModelCallEnd",
        UiEvent::ToolCallStart { .. } => "ToolCallStart",
        UiEvent::ToolCallEnd { .. } => "ToolCallEnd",
        UiEvent::SubAgentSpawned { .. } => "SubAgentSpawned",
        UiEvent::SubAgentFinished { .. } => "SubAgentFinished",
        UiEvent::Step { .. } => "Step",
        UiEvent::ModelReasoning { .. } => "ModelReasoning",
    }
}

// R1: a [tool call, final answer] mission emits one Start and one End
// per call the MissionResult counts.
#[test]
fn m18_every_counted_call_emits_start_and_end() {
    let _guard = ENV_LOCK.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let dir = std::env::temp_dir().join("ui-events-m18");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let answer = dir.join("run").join("work").join("answer.txt");
    std::fs::write(
        dir.join("script.jsonl"),
        format!(
            "{{\"tool\":\"answer.write\",\"args\":{{\"path\":\"{}\",\"content\":\"A\"}}}}\nRead the result. ## Done - lexer fixed, tests green.\n",
            answer.display()
        ),
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
        false, Some(3),
        None,
        None,
    )
    .unwrap();
    let events = std::sync::Arc::new(std::sync::Mutex::new(Vec::<UiEvent>::new()));
    let e2 = events.clone();
    s.set_ui_sink(Box::new(move |ev| e2.lock().unwrap().push(ev)));
    let r = s.run_goal("audit the lexer trailing token").unwrap();

    let evs = events.lock().unwrap();
    let seq: Vec<&str> = evs.iter().map(kind).collect();
    let starts = seq.iter().filter(|k| **k == "ModelCallStart").count();
    let ends = seq.iter().filter(|k| **k == "ModelCallEnd").count();
    assert_eq!(
        starts, r.model_calls as usize,
        "one ModelCallStart per counted call (steps={}, seq={seq:?})",
        r.steps
    );
    assert_eq!(
        ends, r.model_calls as usize,
        "one ModelCallEnd per counted call - the HUD derives its live \
         count from these, so a missing End is a wrong HUD (seq={seq:?})"
    );
}
