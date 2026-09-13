//! REPL UI batch 1 (Eric 2026-09-08: "just fix the gaps", SOTA-REPL UI
//! investigation): hs-repl paints missions as raw, unstyled text - model
//! deltas straight to stderr, tool calls invisible until the result JSON,
//! zero color, no working indicator. pi/omp (the bar) paint tool-call
//! cards with timing, semantic color, and an ambient status line.
//!
//! Contract: the loop exposes a TYPED UI event stream (no print-scraping)
//! and hs-loop ships a Painter that renders it with semantic ANSI color:
//! a tool call becomes a card - colored header with plugin + key arg,
//! trimmed output, and an ok/fail + timing trailer.

use hs_loop::repl::ReplSession;
use hs_loop::uipaint::{Painter, UiEvent};

const ANSWER: &str = env!("CARGO_BIN_EXE_hs-plugin-answer");
const TERMEXEC: &str = env!("CARGO_BIN_EXE_hs-plugin-termexec");
const SCRIPTED: &str = env!("CARGO_BIN_EXE_hs-plugin-scripted");

fn scripted_config(dir: &tempfile::TempDir) -> std::path::PathBuf {
    let config = dir.path().join("hairspring.toml");
    std::fs::write(
        &config,
        format!(
            r#"
[[tools]]
name = "answer.write"
command = ["{ANSWER}"]
subjects = ["*"]

[[tools]]
name = "term.exec"
command = ["{TERMEXEC}"]
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
fn loop_emits_typed_ui_events_for_a_mission() {
    let dir = tempfile::tempdir().unwrap();
    let log = tempfile::tempdir().unwrap();
    let config = scripted_config(&dir);

    // One tool call, then a prose finish.
    let script = dir.path().join("s.jsonl");
    std::fs::write(
        &script,
        serde_json::json!({"tool":"term.exec","args":{"command":"echo UI-CARD-PROBE"}}).to_string(),
    )
    .unwrap();
    unsafe { std::env::set_var("HS_SEQMODEL_SCRIPT", &script) };

    let events = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let sink_events = events.clone();
    let mut session = ReplSession::load(&config, log.path(), true, Some(2)).unwrap();
    session.set_ui_sink(Box::new(move |ev: UiEvent| {
        sink_events.lock().unwrap_or_else(std::sync::PoisonError::into_inner).push(ev);
    }));
    let _ = session.run_goal("paint my tool call");

    let evs = events.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let kinds: Vec<&'static str> = evs
        .iter()
        .map(|e| match e {
            UiEvent::ModelCallStart { .. } => "model_start",
            UiEvent::ModelCallEnd { .. } => "model_end",
            UiEvent::ToolCallStart { .. } => "tool_start",
            UiEvent::ToolCallEnd { .. } => "tool_end",
            UiEvent::SubAgentSpawned { .. } => "agent_spawn",
            UiEvent::SubAgentFinished { .. } => "agent_finish",
            UiEvent::Step { .. } => "step",
            UiEvent::ModelReasoning { .. } => "reasoning",
        })
        .collect();
    assert!(
        kinds.contains(&"tool_start") && kinds.contains(&"tool_end"),
        "mission emitted no typed tool-call UI events, got {kinds:?}"
    );
    let start = evs.iter().find_map(|e| match e {
        UiEvent::ToolCallStart { plugin, args_summary } => Some((plugin.clone(), args_summary.clone())),
        _ => None,
    });
    let (plugin, args) = start.expect("a ToolCallStart");
    assert_eq!(plugin, "term.exec");
    assert!(args.contains("UI-CARD-PROBE"), "args summary carries the command, got {args:?}");
    let end = evs.iter().find_map(|e| match e {
        UiEvent::ToolCallEnd { ok, elapsed_ms, .. } => Some((*ok, *elapsed_ms)),
        _ => None,
    });
    let (ok, _ms) = end.expect("a ToolCallEnd");
    assert!(ok, "echo succeeds");
}

#[test]
fn painter_renders_a_colored_tool_card() {
    let mut out: Vec<u8> = Vec::new();
    {
        let mut p = Painter::new(&mut out, true /* color */);
        p.handle(&UiEvent::ToolCallStart {
            plugin: "term.exec".into(),
            args_summary: "echo UI-CARD-PROBE".into(),
        });
        p.handle(&UiEvent::ToolCallEnd {
            plugin: "term.exec".into(),
            ok: true,
            output_summary: "UI-CARD-PROBE".into(),
            elapsed_ms: 3,
        });
    }
    let s = String::from_utf8(out).unwrap();
    assert!(s.contains("\x1b["), "colored output carries ANSI SGR, got: {s:?}");
    assert!(s.contains("term.exec"), "card names the plugin: {s:?}");
    assert!(s.contains("echo UI-CARD-PROBE"), "card shows the key arg: {s:?}");
    assert!(s.contains("3ms") || s.contains("3 ms"), "card shows timing: {s:?}");
    assert!(s.contains("UI-CARD-PROBE"), "card shows trimmed output: {s:?}");
}

#[test]
fn painter_plain_mode_has_no_ansi() {
    let mut out: Vec<u8> = Vec::new();
    {
        let mut p = Painter::new(&mut out, false /* no color: piped */);
        p.handle(&UiEvent::ToolCallStart {
            plugin: "term.exec".into(),
            args_summary: "echo hi".into(),
        });
        p.handle(&UiEvent::ToolCallEnd {
            plugin: "term.exec".into(),
            ok: true,
            output_summary: "hi".into(),
            elapsed_ms: 1,
        });
    }
    let s = String::from_utf8(out).unwrap();
    assert!(!s.contains("\x1b["), "plain mode stays ANSI-free: {s:?}");
    assert!(s.contains("term.exec") && s.contains("echo hi"));
}
