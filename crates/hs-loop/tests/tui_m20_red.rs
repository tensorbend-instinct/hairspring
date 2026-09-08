//! REPL UI gap #10 M20: the done line lands BEFORE the mission's final
//! answer. Live proof: /tmp/tui-proof-m19/cap1-elided.ansi (and the M17
//! capture) render "── done: 1 steps, 2 calls" ABOVE "Read the result.
//! ## Done ...". Not a thread race: the bin's Done arm pushes the done
//! line and only then commits the held answer tail - deterministic
//! wrong order whenever prose is in flight at mission end (always,
//! under M19's hold-until-disposition). pi/omp never print a summary
//! above the answer it summarizes.
//!
//! Contract: one TuiState method owns mission-end sequencing - the
//! held answer commits FIRST, the done line follows it.

use hs_loop::tui::TuiState;

fn transcript_text(st: &TuiState) -> String {
    st.transcript
        .iter()
        .map(|l| l.spans.iter().map(|s| s.content.to_string()).collect::<String>())
        .collect::<Vec<_>>()
        .join("\n")
}

// R1: with prose still held at mission end, the answer commits before
// the done line that summarizes it.
#[test]
fn m20_done_line_follows_the_answer() {
    let mut st = TuiState::default();
    st.on_answer_delta("the final answer prose");
    st.mission_done_report(1, 2, 1400, false);
    let text = transcript_text(&st);
    let a = text.find("the final answer prose").expect("answer committed");
    let d = text.find("done: 1 steps, 2 calls").expect("done line present");
    assert!(a < d, "answer must precede its done line: {text}");
    assert!(st.answer_inflight.is_empty());
}

// R2: the done line keeps its exact M13 shape and the HUD accounting
// (steps added, cost takes the session total, calls untouched - they
// are counted live).
#[test]
fn m20_report_keeps_counters_and_format() {
    let mut st = TuiState::default();
    st.mission_done_report(3, 3, 3500, true);
    let text = transcript_text(&st);
    assert!(text.contains("done: 3 steps, 3 calls, $0.0035 (budget-killed)"), "format: {text}");
    assert_eq!(st.total_steps, 3);
    assert_eq!(st.total_cost_micros, 3500);
    assert_eq!(st.total_model_calls, 0, "calls are live-counted only (M13)");
    assert_eq!(st.missions_run, 1);
}
