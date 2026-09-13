//! Eric 2026-09-10 (iMessage, via parent): resume-at-cap UX + palette cap
//! control + activity visibility.
//! (a) Resuming a mission that died at the step cap must surface WHY and
//!     the natural fix (raise the cap through the palette).
//! (b) Cap values (steps, wall clock, budget, critic caps) are viewable
//!     and changeable live from the command palette.
//! (c) While the model works, the surface shows what it is doing: the
//!     provider's real reasoning text (never fabricated), the current
//!     step out of the cap, and the action in flight.

use hs_loop::tui::{self, TuiState};
use hs_loop::uipaint::UiEvent;

/// (c1) The provider's reasoning renders as visible transcript content,
/// dim so it never reads as the model's answer.
#[test]
fn reasoning_renders_dim_in_transcript() {
    let mut st = TuiState::default();
    st.on_ui_event(&UiEvent::ModelReasoning {
        text: "the grep found nothing, so the file must not exist yet".to_string(),
    });
    let spans = st.transcript_spans_tail();
    let flat: String = spans.iter().map(|(t, _)| t.clone()).collect();
    assert!(
        flat.contains("the grep found nothing"),
        "reasoning visible: {flat}"
    );
    assert!(
        spans.iter().any(|(_, style)| style.contains("Dim")),
        "reasoning renders dim: {spans:?}"
    );
}

/// (c2) The activity surface names the current step out of the cap and
/// what is happening right now.
#[test]
fn activity_line_names_step_and_action() {
    let mut st = TuiState::default();
    st.on_ui_event(&UiEvent::Step {
        step: 3,
        max_steps: Some(50),
    });
    st.on_ui_event(&UiEvent::ModelCallStart {
        model: "deepseek".to_string(),
    });
    let a = st.activity_line();
    assert!(a.contains("step 3/50"), "step in activity: {a}");
    assert!(a.contains("deepseek"), "model in activity: {a}");
    st.on_ui_event(&UiEvent::ToolCallStart {
        plugin: "term.exec".to_string(),
        args_summary: "cargo test".to_string(),
    });
    let a = st.activity_line();
    assert!(a.contains("step 3/50"), "step survives tool start: {a}");
    assert!(a.contains("term.exec"), "tool in activity: {a}");
}

/// (c3) A reasoning event is offered only when the provider returned
/// real reasoning text - never fabricated from nothing.
#[test]
fn reasoning_event_only_for_real_text() {
    assert!(hs_loop::uipaint::reasoning_event("").is_none());
    assert!(hs_loop::uipaint::reasoning_event("   ").is_none());
    let ev = hs_loop::uipaint::reasoning_event("checking the ledger first").unwrap();
    match ev {
        UiEvent::ModelReasoning { text } => assert!(text.contains("checking the ledger")),
        other => panic!("wrong event: {other:?}"),
    }
}

/// (b1) /caps parses: bare is a query, key+value is a set.
#[test]
fn caps_command_parses() {
    assert_eq!(tui::parse_caps_command("/caps"), Some(tui::CapsCmd::Query));
    assert_eq!(
        tui::parse_caps_command("/caps steps 100"),
        Some(tui::CapsCmd::Set { key: "steps".into(), value: "100".into() })
    );
    assert_eq!(
        tui::parse_caps_command("/caps budget 2.50"),
        Some(tui::CapsCmd::Set { key: "budget".into(), value: "2.50".into() })
    );
    assert_eq!(
        tui::parse_caps_command("/caps critic-steps 24"),
        Some(tui::CapsCmd::Set { key: "critic-steps".into(), value: "24".into() })
    );
    assert_eq!(tui::parse_caps_command("/caps bogus"), None);
}

/// (b2) The caps listing names every cap the config holds.
#[test]
fn caps_listing_names_every_cap() {
    let snap = tui::CapsSnapshot {
        steps: Some(50),
        wall_secs: None,
        budget_micros: Some(10_000_000),
        critic_steps: 12,
        critic_wall_secs: 600,
        critic_budget_micros: 1_000_000,
    };
    let l = tui::format_caps_listing(&snap);
    for needle in ["steps 50", "wall", "budget $10.0000 (billed spend)", "critic steps 12", "critic wall 600", "critic budget $1.00"] {
        assert!(l.contains(needle), "missing {needle}: {l}");
    }
}

/// (a) Resuming a capped mission tells the user what happened and the
/// natural fix; a clean outcome stays quiet.
#[test]
fn capped_resume_notice_points_at_caps() {
    let n = hs_loop::repl::capped_resume_notice(Some("steps_exhausted")).expect("capped notice");
    assert!(n.contains("steps_exhausted"), "{n}");
    assert!(n.contains("/caps"), "points at the palette: {n}");
    assert!(hs_loop::repl::capped_resume_notice(Some("verified")).is_none());
    assert!(hs_loop::repl::capped_resume_notice(None).is_none());
}

/// (c4) Loop level: a mission emits a Step event per loop iteration
/// and one ModelReasoning event carrying the provider's exact
/// reasoning_content - the live sink path the TUI consumes.
#[test]
fn mission_emits_step_and_reasoning_events() {
    let dir = std::env::temp_dir().join("caps-activity-loop");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("script.jsonl"),
        concat!(
            "{\"completion\": \"Looking at the ledger first.\", \"reasoning\": \"the grep found nothing, so the file must not exist yet\"}\n",
            "## Done - ledger checked\n",
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
        false, Some(2),
        None,
        None,
    )
    .unwrap();
    let events = std::sync::Arc::new(std::sync::Mutex::new(Vec::<UiEvent>::new()));
    let e2 = events.clone();
    s.set_ui_sink(Box::new(move |ev| e2.lock().unwrap().push(ev)));
    let r = s.run_goal("check the ledger").unwrap();
    assert!(r.steps >= 1, "fixture ran: {:?}", r.outcome);
    let evs = events.lock().unwrap();
    let steps: Vec<u32> = evs
        .iter()
        .filter_map(|e| match e {
            UiEvent::Step { step, max_steps } => {
                assert_eq!(*max_steps, Some(2), "the session cap rides the event");
                Some(*step)
            }
            _ => None,
        })
        .collect();
    assert_eq!(steps, vec![1, 2], "one Step event per iteration: {steps:?}");
    let reasoning: Vec<&str> = evs
        .iter()
        .filter_map(|e| match e {
            UiEvent::ModelReasoning { text } => Some(text.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(
        reasoning,
        vec!["the grep found nothing, so the file must not exist yet"],
        "the provider's exact reasoning, once: {reasoning:?}"
    );
}
