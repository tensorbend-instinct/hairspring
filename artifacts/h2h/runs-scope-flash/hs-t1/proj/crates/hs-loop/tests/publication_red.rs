//! RED-first pin for the gate-8 publication harness (v5: "Beat the
//! published single-session incumbent baseline by 2x on the defined
//! benchmark suite (gate 8). A gate, not a promise." + checklist 7.9:
//! the full `T_mission` decomposition is published per run, win or lose).
//!
//! v1: the defined token-family suite runs in TWO arms on the real
//! pipeline (real checker, mechanical verifier) - the single-session
//! incumbent (fresh session per attempt, no feedback channel, restart
//! from scratch on failure, failed work paid twice) and hairspring (one
//! continuous session with verifier feedback). The publication compares
//! them in decomposition TERMS and writes the artifact with the
//! operationalization verbatim, win or lose.

use hs_loop::publication::{run_publication, Verdict};

#[test]
fn v1_two_arm_publication_is_priced_in_terms_and_published() {
    let dir = std::env::temp_dir().join("publication-v1");
    let _ = std::fs::remove_dir_all(&dir);
    let p = run_publication(&dir, 2, 2).unwrap();

    // both arms complete every task
    assert_eq!(p.hairspring.passed, 2, "hairspring arm: {p:?}");
    assert_eq!(p.incumbent.passed, 2, "incumbent arm: {p:?}");

    // the incumbent restarted from scratch twice per task and its
    // attempts are all measured; hairspring needed one session
    assert_eq!(p.incumbent.restarts, 4);
    assert_eq!(p.hairspring.restarts, 0);
    assert_eq!(p.incumbent.tasks[0].attempts.len(), 3);
    assert_eq!(p.hairspring.tasks[0].attempts.len(), 1);

    // the stuck-repeat term only exists where there is no feedback
    // channel: the incumbent replays the identical wrong call until its
    // step budget burns out
    assert!(
        p.incumbent.total.s_stuck_repeats >= 8,
        "incumbent decomposition: {:?}",
        p.incumbent.total
    );
    assert_eq!(p.hairspring.total.s_stuck_repeats, 0);

    // the comparison is priced in decomposition terms against the 2x
    // gate, not wall microseconds
    assert!(
        p.incumbent.priced_steps > p.incumbent.raw_steps,
        "restart redo must be priced: {p:?}"
    );
    assert!(
        p.incumbent.priced_steps >= 2 * p.hairspring.priced_steps,
        "incumbent={} hairspring={}",
        p.incumbent.priced_steps,
        p.hairspring.priced_steps
    );
    assert_eq!(p.verdict, Verdict::Win);

    // the artifact publishes the operationalization verbatim, win or lose
    let path = p.write(&dir.join("artifact")).unwrap();
    let body = std::fs::read_to_string(&path).unwrap();
    for needle in [
        "OPERATIONALIZATION (verbatim):",
        "resets every session",
        "published win or lose",
        "ARM incumbent:",
        "ARM hairspring:",
        "T_mission decomposition",
        "target=2x",
        "verdict=WIN",
    ] {
        assert!(body.contains(needle), "artifact missing {needle}:\n{body}");
    }
}
