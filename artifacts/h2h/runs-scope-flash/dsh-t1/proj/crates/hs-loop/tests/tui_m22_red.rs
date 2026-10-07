//! REPL UI gap #10 M22: the live done line hides the mission outcome.
//! A mission that ends `steps_exhausted` / `ratchet_capped` /
//! `verifier_malfunction` / `harness_error` prints the exact same
//! "done: N steps, N calls, $X" line as a verified one - failure is
//! invisible live. Worse, the RESUME backfill (M14) prints the
//! outcome - "done: ... $0.0014 (verified)" - so a resumed session
//! shows MORE information than the live one did. Same screen, two
//! formats, and the live one silently drops the single most important
//! fact about how the mission ended. pi/omp surface failure loudly.
//!
//! Contract: `mission_done_report` takes the mission outcome and the
//! live line matches the backfill format byte-for-byte:
//! "── done: {steps} steps, {calls} calls, {cost} ({outcome})".

fn transcript_text(st: &hs_loop::tui::TuiState) -> String {
    st.transcript
        .iter()
        .map(|l| l.spans.iter().map(|s| s.content.to_string()).collect::<String>())
        .collect::<Vec<_>>()
        .join("\n")
}

// R1: every outcome is visible on the live done line, not only
// budget-killed. Failure endings must read as failures.
#[test]
fn m22_done_line_carries_outcome() {
    let mut st = hs_loop::tui::TuiState::default();
    st.mission_done_report(2, 2, 1400, 1400, "verified");
    st.mission_done_report(3, 3, 2100, 3500, "steps_exhausted");
    st.mission_done_report(1, 4, 900, 4400, "ratchet_capped");
    let text = transcript_text(&st);
    assert!(
        text.contains("done: 2 steps, 2 calls, $0.0014 (verified)"),
        "verified outcome shown: {text}"
    );
    assert!(
        text.contains("done: 3 steps, 3 calls, $0.0021 (steps_exhausted)"),
        "a failed mission must read as failed live: {text}"
    );
    assert!(
        text.contains("done: 1 steps, 4 calls, $0.0009 (ratchet_capped)"),
        "capped pass is not byte-identical to an audited one: {text}"
    );
}

// R2: live line and resume-backfill line are the SAME format - the
// backfill (tui.rs backfill_transcript) already emits
// "── done: {steps} steps, {calls} calls, {cost} ({outcome})".
#[test]
fn m22_live_matches_backfill_format() {
    let mut st = hs_loop::tui::TuiState::default();
    st.mission_done_report(5, 7, 4200, 4200, "verifier_malfunction");
    let text = transcript_text(&st);
    assert!(
        text.contains("\u{2500}\u{2500} done: 5 steps, 7 calls, $0.0042 (verifier_malfunction)"),
        "live format identical to backfill: {text}"
    );
}
