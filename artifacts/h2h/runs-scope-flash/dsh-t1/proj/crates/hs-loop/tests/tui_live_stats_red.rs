//! RED (2026-10-06 usability comparison): HAIRSPRING showed no turn clock,
//! tok/s or cache hit; a 9-minute silent loop looked like a hang. The
//! always-on footer now carries elapsed, steps, calls, tok/s and cache hit,
//! computed only from what the provider reported and real clocks.

use hs_loop::tui::TuiState;
use hs_loop::uipaint::UiEvent as U;
use std::time::{Duration, Instant};

fn start(st: &mut TuiState, t: Instant) {
    st.on_ui_event_at(&U::Step { step: 1, max_steps: None }, t);
    st.on_ui_event_at(&U::ModelCallStart { model: "deepseek-v4-pro".into() }, t);
}

#[test]
fn footer_shows_turn_clock_steps_calls_toks_and_cache() {
    let mut st = TuiState::default();
    let t0 = Instant::now();
    start(&mut st, t0);
    st.on_ui_event_at(
        &U::ModelCallEnd {
            model: "deepseek-v4-pro".into(),
            input_tokens: 10_000,
            output_tokens: 680,
            cost_usd_micros: 1,
        },
        t0 + Duration::from_secs(5),
    );
    st.on_ui_event_at(
        &U::ModelCallCache { cached_tokens: 9_500, input_tokens: 10_000 },
        t0 + Duration::from_secs(5),
    );
    let line = st.footer_stats(t0 + Duration::from_secs(112));
    assert!(line.contains("1m52s"), "turn clock: {line}");
    assert!(line.contains("step 1"), "{line}");
    assert!(line.contains("1 call"), "{line}");
    assert!(line.contains("136 tok/s"), "680 tokens / 5s: {line}");
    assert!(line.contains("cache 95%"), "{line}");
}

#[test]
fn unknown_stats_are_omitted_not_invented() {
    let st = TuiState::default();
    assert_eq!(st.footer_stats(Instant::now()), "");
}

#[test]
fn clock_stops_when_the_mission_is_done() {
    let mut st = TuiState::default();
    let t0 = Instant::now();
    start(&mut st, t0);
    st.mission_done(3, 0);
    let line = st.footer_stats(t0 + Duration::from_secs(500));
    assert!(!line.contains("8m"), "no running clock after done: {line}");
}

#[test]
fn live_footer_renders_the_stats() {
    // the rendered HUD row carries them, not just the helper
    let mut st = TuiState::default();
    let t0 = Instant::now();
    start(&mut st, t0);
    st.on_ui_event_at(
        &U::ModelCallEnd { model: "deepseek-v4-pro".into(), input_tokens: 1, output_tokens: 100, cost_usd_micros: 1 },
        t0 + Duration::from_secs(1),
    );
    let hud = st.footer_text(t0 + Duration::from_secs(3));
    assert!(hud.contains("3s") && hud.contains("tok/s"), "{hud}");
}
