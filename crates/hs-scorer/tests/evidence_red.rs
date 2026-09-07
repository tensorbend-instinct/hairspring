//! GATE 9c (spec v5; Yan et al., arXiv:2609.01481 - v1 preprint, imports are
//! design rules): evidence state is a FIRST-CLASS record, separate from
//! artifact state. "An artifact says what exists; evidence says what is
//! known about it." Regression bookkeeping is tracked, not overwritten: "a
//! claim verified at N that fails at M becomes a regression record pointing
//! at both events, so 'it worked before' is a query over the record."
//!
//! Ownership: the scorer observes the assays, so the scorer owns claim
//! bookkeeping (single-writer discipline on its stream). Evidence state is
//! PROJECTED from the canonical log - a read path, no parallel store.
//!
//! Falsifiable: a same-named candidate that passes then later fails must
//! surface as a regression record naming BOTH event refs; without tracked
//! bookkeeping the failure would silently overwrite the earlier claim.

use hs_core::EventKind;
use hs_scorer::evidence::{ClaimKind, ClaimStatus};
use hs_scorer::*;

fn suite() -> TaskSuite {
    TaskSuite::new(
        "tokens",
        (0..3)
            .map(|i| Task::new(format!("T{i}"), format!("S{i}")))
            .collect(),
    )
}

#[test]
fn verified_then_failed_is_a_tracked_regression_pointing_at_both_events() {
    let dir = tempfile::tempdir().unwrap();
    let mut scorer = Scorer::new(dir.path(), ScorerConfig::default()).unwrap();

    // candidate passes: claim verified at event N
    let good = Artifact::by_rule(|t: &Task| Some(t.secret().to_string()));
    scorer
        .tier01_execution(&Candidate::new("cand", good), &suite())
        .unwrap();
    let ev = scorer.evidence_state();
    assert_eq!(ev.len(), 1, "one claim per subject: {ev:?}");
    assert_eq!(ev[0].subject, "cand");
    assert_eq!(ev[0].kind, ClaimKind::VerifiedClaim);
    assert_eq!(ev[0].status, ClaimStatus::Open);
    let n = ev[0]
        .verified_at
        .expect("verified claim carries its event ref");

    // same subject later fails the same suite: regression, tracked
    let broken = Artifact::by_rule(|_| None);
    scorer
        .tier01_execution(&Candidate::new("cand", broken), &suite())
        .unwrap();
    let ev = scorer.evidence_state();
    assert_eq!(
        ev.len(),
        1,
        "still one claim - tracked, not duplicated: {ev:?}"
    );
    assert_eq!(
        ev[0].kind,
        ClaimKind::Regression,
        "verified-then-failed = regression"
    );
    assert_eq!(
        ev[0].verified_at,
        Some(n),
        "regression keeps the verify ref"
    );
    let m = ev[0]
        .regressed_at
        .expect("regression carries the failing event ref");
    assert_ne!(n, m, "two distinct events");

    // the regression is ON THE LOG as a regression event naming both refs
    let reg = scorer
        .log_events()
        .into_iter()
        .filter(|e| e.kind == EventKind::Regression)
        .count();
    assert_eq!(reg, 1, "exactly one regression event on the canonical log");

    // the proposer's read: what is verified, what is failing, what regressed
    assert!(scorer.open_failures().is_empty());
    assert_eq!(scorer.regressions().len(), 1);
    assert_eq!(
        scorer.verified_claims().len(),
        0,
        "regressed is no longer verified"
    );
}

#[test]
fn a_fresh_failure_without_prior_verification_is_an_open_failure() {
    let dir = tempfile::tempdir().unwrap();
    let mut scorer = Scorer::new(dir.path(), ScorerConfig::default()).unwrap();
    let broken = Artifact::by_rule(|_| None);
    scorer
        .tier01_execution(&Candidate::new("cand", broken), &suite())
        .unwrap();
    let ev = scorer.evidence_state();
    assert_eq!(ev[0].kind, ClaimKind::OpenFailure);
    assert_eq!(scorer.open_failures().len(), 1);
    assert_eq!(
        scorer.regressions().len(),
        0,
        "nothing verified before: no regression"
    );
}
