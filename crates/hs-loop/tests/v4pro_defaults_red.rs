//! RED: v4-pro-era defaults - Eric's 2026-09-10 ruling: "Just use v4 pro
//! for now, raise its mission cap if it's better per your results".
//!
//! Measured basis (cap-escalation matrix, live TUI, 2026-09-10):
//! - flash critic capped WITHOUT verdict at 8/16/24 steps (B cells);
//!   greened in 32 of 48 (C48, /tmp/tui-proof-live/C48).
//! - v4-pro critic verdicted under cap 48 with a higher-quality refute
//!   (P48, /tmp/tui-proof-live/P48): caught wrong-workdir placement
//!   flash's green chain never saw.
//! - v4-pro verdict wall: 338s event latency (up to ~15 min by ts delta);
//!   a 600s wall default can cut a slow v4-pro verdict mid-flight.
//! - the mission model (25-step default) hit steps_exhausted in P48 right
//!   as its corrected final submit landed: the mission cap, not the
//!   critic cap, was the binding constraint.

use hs_loop::critic::RefuteConfig;

#[test]
fn critic_default_steps_cover_measured_green() {
    let d = RefuteConfig::default();
    assert_eq!(
        d.max_steps, 48,
        "critic default must cover the measured 32-step flash green (C48) and v4-pro verdicts (P48)"
    );
    assert_eq!(
        d.wall_secs, 1800,
        "v4-pro verdict measured 338s-879s wall; 600s default can cut it mid-flight"
    );
}

#[test]
fn deepseek_default_model_is_v4_pro() {
    assert_eq!(
        hs_loop::realmodel::deepseek().default_model,
        "deepseek-v4-pro",
        "Eric 2026-09-10: 'just use v4 pro for now'"
    );
}

#[test]
fn mission_default_steps_survive_refute_history() {
    assert_eq!(
        hs_loop::DEFAULT_MISSION_MAX_STEPS, 50,
        "25 exhausted in P48 before the corrected submit could verdict"
    );
}
