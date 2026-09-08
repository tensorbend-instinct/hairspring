//! REPL UI gap #10 M24: the HUD pluralizes "mission" but not "step" -
//! a one-step session reads "1 mission · 1 steps" (live proof:
//! cap20). Same line, two rules; the counter the user reads most
//! should not have a grammar bug.

// R1: singular and plural agree with the count for BOTH counters.
#[test]
fn r1_hud_pluralizes_steps() {
    let mut st = hs_loop::tui::TuiState {
        missions_run: 1,
        total_steps: 1,
        ..Default::default()
    };
    let hud = st.hud_line();
    assert!(hud.contains("1 mission"), "singular mission: {hud}");
    assert!(hud.contains("1 step "), "singular step, not '1 steps': {hud}");
    st.total_steps = 2;
    let hud = st.hud_line();
    assert!(hud.contains("2 steps"), "plural steps: {hud}");
}
