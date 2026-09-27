//! Default critic mission caps are opt-in. Provider and command watchdogs remain safety timeouts.

use hs_loop::critic::RefuteConfig;

#[test]
fn critic_caps_are_opt_in_by_default() {
    let d = RefuteConfig::default();
    assert_eq!(
        d.max_steps, None,
        "critic step cap is off by default"
    );
    assert_eq!(d.wall_secs, None, "critic wall cap is off by default");
    assert_eq!(d.budget_micros, None, "critic spend cap is off by default");
}

#[test]
fn deepseek_default_model_is_v4_pro() {
    assert_eq!(
        hs_loop::realmodel::deepseek().default_model,
        "deepseek-v4-pro",
        "Eric 2026-09-10: 'just use v4 pro for now'"
    );
}

/// Eric 2026-09-12 superseded the 50-step user-facing default: a fresh
/// rig arms NO step cap ("the caps should start with no caps... settable
/// during setup or within the TUI"). The constant survives only as the
/// bench binaries' explicit default; the no-cap default itself is proven
/// end to end in `caps_default_red.rs`.
#[test]
fn mission_step_cap_is_opt_in_not_default() {
    assert_eq!(
        hs_loop::DEFAULT_MISSION_MAX_STEPS, 50,
        "bench binaries keep an explicit cap; the REPL default is now uncapped (caps_default_red)"
    );
}
