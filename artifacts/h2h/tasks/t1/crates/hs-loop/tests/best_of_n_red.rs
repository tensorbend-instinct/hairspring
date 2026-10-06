//! RED-first pins for the 8.5 best-of-N published run (spec: "The
//! best-of-N baseline - every task family carries an endpoint-wise
//! best-of-N envelope of isolated agents with identical decision
//! opportunities. Any claimed collective or evolutionary advantage must
//! beat matched independent search, not just add samples."; checklist
//! 8.5: endpoint-wise envelope + published comparison artifact, win or
//! lose; the published RUN rides with 7.9's two-arm harness).
//!
//! The envelope is the STRENGTHENED gate-8 baseline: N independent
//! incumbent-style isolates (fresh session per attempt, no cross-session
//! memory, no feedback channel), matched seeds (identical task secrets)
//! and identical decision opportunities (same per-attempt budgets). The
//! hairspring candidate must beat the ENVELOPE (endpoint-wise best
//! isolate per task), not any single isolate.
//!
//! Metric where the win lives, named in the artifact: pass endpoints
//! first (a task the envelope cannot pass at all is a task matched
//! search lost), priced steps in `T_mission` terms against the 2x gate
//! second. Published win or lose.
//!
//! v1: envelope semantics + published artifact on the real pipeline
//!     (uniform draw: every isolate can pass every task; the truthful
//!     verdict on this fixture is LOSE and the artifact must print it).
//! v2: can-lose - strengthened isolates, no doom loop, verdict LOSE.
//! v3: can-win - specialist draw (matched search where isolates are
//!     seeded partial coverage, `gate7_proof_3` shape): a task NO isolate
//!     covers is an endpoint the candidate wins outright; the verdict
//!     flips to WIN and the artifact prints it.

use hs_loop::publication::{run_best_of_n_publication, IsolateDraw, Verdict};

#[test]
fn v1_envelope_semantics_and_published_artifact() {
    let dir = std::env::temp_dir().join("best-of-n-v1");
    let _ = std::fs::remove_dir_all(&dir);
    let p = run_best_of_n_publication(&dir, 3, 2, 3, IsolateDraw::Uniform).unwrap();

    // every arm completes every task
    assert_eq!(p.hairspring.passed, 3);
    assert_eq!(p.incumbent.passed, 3);
    assert_eq!(p.isolates.len(), 3);
    for iso in &p.isolates {
        assert_eq!(iso.passed, 3, "isolate {:?}: {iso:?}", iso.name);
        assert_eq!(iso.tasks.len(), 3);
    }

    // isolate outcomes actually VARY per (task, isolate): not all
    // isolates pay the same priced steps on every task (otherwise the
    // envelope is a relabeled single run - degenerate)
    let t0: Vec<u64> = p
        .isolates
        .iter()
        .map(|iso| iso.tasks[0].raw_steps_paid())
        .collect();
    assert!(
        t0.iter().any(|&s| s != t0[0]),
        "isolate outcomes must vary for the envelope to mean anything: {t0:?}"
    );

    // endpoint-wise envelope, recomputed INDEPENDENTLY in the test from
    // the isolate reports (no shared code path with production math):
    // per task the envelope credits the best isolate endpoint
    // (fewest priced steps among passing isolates)
    let mut expect_total = 0u64;
    for ti in 0..3 {
        let best = p
            .isolates
            .iter()
            .map(|iso| iso.tasks[ti].raw_steps_paid())
            .min()
            .unwrap();
        expect_total += best;
        assert_eq!(p.envelope.tasks[ti].priced_steps, best, "task {ti}");
        assert!(p.envelope.tasks[ti].passed);
    }
    assert_eq!(p.envelope.priced_steps, expect_total);
    assert_eq!(p.envelope.passed, 3);

    // the named metric, recomputed independently: pass endpoints first,
    // priced steps against the 2x gate second
    let expect_speedup = p.envelope.priced_steps as f64 / p.hairspring.priced_steps as f64;
    assert!((p.envelope_speedup - expect_speedup).abs() < 1e-9);
    let expect_verdict = if p.hairspring.passed > p.envelope.passed
        || (p.hairspring.passed == p.envelope.passed && expect_speedup >= 2.0)
    {
        Verdict::Win
    } else {
        Verdict::Lose
    };
    assert_eq!(p.envelope_verdict, expect_verdict);

    // the single-incumbent comparison still ships (continuity with B10)
    assert!(p.base_speedup >= 2.0, "base gate-8 win expected: {p:?}");

    // the artifact publishes everything, win or lose
    let path = p.write(&dir.join("artifact")).unwrap();
    let body = std::fs::read_to_string(&path).unwrap();
    for needle in [
        "best-of-N envelope (endpoint-wise",
        "matched decision opportunities",
        "matched seeds",
        "metric=pass_endpoints_then_priced_steps",
        "envelope_priced_steps=",
        "envelope_passed=",
        "isolate_priced_steps=",
        "envelope_speedup=",
        "envelope_verdict=",
        "target=2x",
    ] {
        assert!(body.contains(needle), "artifact missing {needle}:\n{body}");
    }
}

#[test]
fn v2_strengthened_isolates_flip_the_envelope_verdict_to_lose() {
    // can-lose: zero induced failed attempts = every isolate passes
    // first try, no doom loop for the candidate's feedback channel to
    // repair; the envelope of matched isolates beats the candidate on
    // priced steps and the run MUST publish LOSE
    let dir = std::env::temp_dir().join("best-of-n-v2");
    let _ = std::fs::remove_dir_all(&dir);
    let p = run_best_of_n_publication(&dir, 2, 0, 3, IsolateDraw::Uniform).unwrap();
    assert_eq!(p.hairspring.passed, 2);
    for iso in &p.isolates {
        assert_eq!(iso.passed, 2);
    }
    assert!(
        p.envelope_speedup < 2.0,
        "no doom loop anywhere, no 2x: {}",
        p.envelope_speedup
    );
    assert_eq!(p.envelope_verdict, Verdict::Lose, "rig must lose: {p:?}");
    let path = p.write(&dir.join("artifact")).unwrap();
    let body = std::fs::read_to_string(&path).unwrap();
    assert!(
        body.contains("envelope_verdict=LOSE"),
        "the loss prints verbatim:\n{body}"
    );
}

#[test]
fn v3_specialist_draw_flips_the_envelope_verdict_to_win() {
    // can-win: specialist isolates (gate7_proof_3 shape) - matched
    // independent search where each isolate is seeded partial coverage.
    // With n=3 and the (7i+k) mod 4 draw: isolate-1 alone covers task-1,
    // isolate-2 alone covers task-2, and NO isolate covers task-3 - a
    // task matched search structurally loses. The candidate passes all
    // three, so it beats the envelope on pass endpoints outright.
    let dir = std::env::temp_dir().join("best-of-n-v3");
    let _ = std::fs::remove_dir_all(&dir);
    let p = run_best_of_n_publication(&dir, 3, 2, 3, IsolateDraw::Specialist).unwrap();

    // the draw landed as specified (raw evidence, not vibes)
    assert_eq!(p.isolates[0].passed, 0, "isolate-0 covers nothing: {p:?}");
    assert_eq!(p.isolates[1].passed, 1, "isolate-1 covers task-1 only");
    assert_eq!(p.isolates[2].passed, 1, "isolate-2 covers task-2 only");
    assert!(p.isolates[1].tasks[0].passed, "isolate-1 must pass task-1");
    assert!(p.isolates[2].tasks[1].passed, "isolate-2 must pass task-2");

    // envelope: task-1 and task-2 pass (one specialist each), task-3
    // fails - no isolate covers it
    assert!(p.envelope.tasks[0].passed);
    assert_eq!(p.envelope.tasks[0].best_isolate, 1);
    assert!(p.envelope.tasks[1].passed);
    assert_eq!(p.envelope.tasks[1].best_isolate, 2);
    assert!(!p.envelope.tasks[2].passed, "no isolate covers task-3");
    assert_eq!(p.envelope.passed, 2);

    // the candidate passes all three and beats the envelope on the
    // named metric: pass endpoints first
    assert_eq!(p.hairspring.passed, 3);
    assert_eq!(
        p.envelope_verdict,
        Verdict::Win,
        "candidate beats the envelope on endpoints: {p:?}"
    );

    // and the win prints verbatim
    let path = p.write(&dir.join("artifact")).unwrap();
    let body = std::fs::read_to_string(&path).unwrap();
    assert!(
        body.contains("envelope_verdict=WIN"),
        "the win prints verbatim:\n{body}"
    );
    assert!(body.contains("task-3: passed=false"), "the failed endpoint prints:\n{body}");
}
