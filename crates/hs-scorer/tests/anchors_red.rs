//! RED-first pin for the scorer anchor plane (v5 tier-2 hygiene, checklist
//! 8.2 + 8.4):
//!
//! - calibration anchors: a standing, versioned, append-only set of
//!   human-labeled artifacts per domain; judge drift is measured against it
//!   and a drifting judge is DROPPED from the panel (`AnchorResult` events
//!   are substrate events, one per judge, never policy data);
//! - cross-family hygiene: a one-family panel's veto weight is REDUCED and
//!   the tier-2 record says so (`families=1 veto_weight=0.5`);
//! - canary re-anchor: a scorer frozen by canary drift stays frozen while
//!   the corruption persists and lifts ONLY when a re-measured control group
//!   is back under threshold ("frozen for promotion decisions until
//!   re-anchored").

use hs_core::EventKind;
use hs_scorer::{
    Anchor, AnchorSet, Artifact, Candidate, Canary, CanarySuite, ClosureJudge, DriftKind, Judge,
    JudgePanel, Lineage, Scorer, ScorerConfig, Task, TaskSuite,
};

fn generalizer() -> Artifact {
    Artifact::by_rule(|t| Some(t.secret().to_string()))
}

fn wrong_memorizer() -> Artifact {
    Artifact::by_rule(|_| Some("plausible-but-wrong".to_string()))
}

fn heldout_suite() -> TaskSuite {
    TaskSuite::new(
        "token-heldout",
        (0..4)
            .map(|i| Task::new(format!("H{i}"), format!("H-{i}-SECRET")))
            .collect(),
    )
}

fn canary_suite() -> CanarySuite {
    CanarySuite::new(vec![
        Canary::known_good("cg-1", generalizer()),
        Canary::known_bad("cb-1", wrong_memorizer()),
    ])
}

#[test]
fn frozen_scorer_stays_frozen_until_reanchored_clean() {
    let tmp = tempfile::tempdir().unwrap();
    let mut scorer = Scorer::new(tmp.path(), ScorerConfig::default()).unwrap();
    let pin = scorer.pin();
    let heldout = heldout_suite();
    let canaries = canary_suite();

    // healthy control: no drift, no freeze
    let rep = scorer.run_canaries(&canaries, &heldout, &pin).unwrap();
    assert!(!rep.drifted && !scorer.is_frozen());

    // corrupt the held-out evaluator (gamed / degraded scorer)
    scorer.seed_drift_for_test(DriftKind::HeldOutAlwaysPasses);
    let rep = scorer.run_canaries(&canaries, &heldout, &pin).unwrap();
    assert!(rep.drifted, "canary suite must flag the drift");
    assert!(scorer.is_frozen());

    // re-anchor while the corruption persists: measurement still fails,
    // freeze stands
    let rep = scorer.re_anchor(&canaries, &heldout, &pin).unwrap();
    assert!(rep.drifted);
    assert!(
        scorer.is_frozen(),
        "a still-corrupt scorer re-anchors nothing"
    );

    // corruption repaired: re-anchor re-measures the control group and the
    // freeze lifts, recorded as a substrate event
    scorer.clear_drift_for_test();
    let rep = scorer.re_anchor(&canaries, &heldout, &pin).unwrap();
    assert!(!rep.drifted);
    assert!(!scorer.is_frozen(), "clean control group lifts the freeze");
    assert!(
        scorer
            .event_texts()
            .iter()
            .any(|t| t.contains("scorer_reanchored")),
        "the re-anchor is recorded on the log"
    );
    assert!(
        scorer
            .log_events()
            .iter()
            .any(|e| e.kind == EventKind::AnchorResult),
        "anchor outcomes are substrate events"
    );

    // the promotion gate is open again
    let mut lineage = Lineage::new(tmp.path().join("lineage"), "token").unwrap();
    let cand = Candidate::new("good", generalizer());
    let v = scorer.held_out_assay(&cand, &heldout, &pin).unwrap();
    let s0 = scorer.tier01_execution(&cand, &heldout).unwrap();
    let panel = JudgePanel::new(vec![
        Box::new(ClosureJudge::new("j-a", "famA", |_: &Artifact, _: &Task| 0.9)) as Box<dyn Judge>,
    ]);
    let s1 = scorer.tier02_rubric(&cand, &heldout, &panel).unwrap();
    lineage.record(&cand, s0, s1, v.clone());
    lineage
        .promote(&cand, &v, &pin, &scorer)
        .unwrap_or_else(|e| panic!("re-anchored scorer must promote again: {e}"));
    assert_eq!(lineage.champion().unwrap().name(), "good");
}

#[test]
fn anchor_audit_measures_drift_and_drops_drifting_judges() {
    let tmp = tempfile::tempdir().unwrap();
    let mut scorer = Scorer::new(tmp.path(), ScorerConfig::default()).unwrap();

    // standing human-labeled anchor set for the domain
    let mut anchors = AnchorSet::new("token-domain");
    for i in 0..3 {
        anchors.append(Anchor::new(
            Task::new(format!("A{i}"), format!("A-{i}-SECRET")),
            generalizer(),
            1.0,
        ));
    }

    let mut panel = JudgePanel::new(vec![
        Box::new(ClosureJudge::new("accurate", "famA", |_: &Artifact, _: &Task| 1.0))
            as Box<dyn Judge>,
        Box::new(ClosureJudge::new("drifted", "famB", |_: &Artifact, _: &Task| 0.2)),
    ]);

    let audit = scorer.run_anchor_audit(&mut panel, &anchors);
    assert_eq!(audit.domain, "token-domain");
    assert_eq!(audit.version, anchors.version());

    let a = audit
        .results
        .iter()
        .find(|r| r.judge == "accurate")
        .expect("every judge gets a drift measurement");
    let b = audit
        .results
        .iter()
        .find(|r| r.judge == "drifted")
        .expect("every judge gets a drift measurement");
    assert!(a.drift.abs() < 1e-9, "calibrated judge: zero drift");
    assert!(!a.dropped);
    assert!(
        (b.drift - 0.8).abs() < 1e-9,
        "drift = mean |judge - human_label|"
    );
    assert!(b.dropped, "a judge that drifts is re-anchored or dropped");

    // the dropped judge no longer judges
    assert_eq!(panel.judge_names(), vec!["accurate".to_string()]);

    // one substrate event per judge, with the measurement and the outcome
    let texts = scorer.event_texts();
    let anchor_events: Vec<&String> = texts
        .iter()
        .filter(|t| t.contains("anchor_result"))
        .collect();
    assert_eq!(anchor_events.len(), 2, "one AnchorResult per judge");
    assert!(anchor_events
        .iter()
        .any(|t| t.contains("judge=drifted") && t.contains("outcome=dropped")));
    assert!(anchor_events
        .iter()
        .any(|t| t.contains("judge=accurate") && t.contains("outcome=kept")));
}

#[test]
fn anchor_set_is_versioned_and_append_only() {
    let mut set = AnchorSet::new("token-domain");
    assert_eq!(set.version(), 0);
    assert_eq!(set.domain(), "token-domain");
    set.append(Anchor::new(
        Task::new("t0".into(), "s0".into()),
        generalizer(),
        1.0,
    ));
    set.append(Anchor::new(
        Task::new("t1".into(), "s1".into()),
        wrong_memorizer(),
        0.1,
    ));
    assert_eq!(set.version(), 2, "every append bumps the version");
    assert_eq!(set.anchors().len(), 2, "no anchor is ever removed");
    assert!((set.anchors()[0].human_label - 1.0).abs() < 1e-9);
    assert!((set.anchors()[1].human_label - 0.1).abs() < 1e-9);
}

#[test]
fn single_family_panel_veto_weight_is_reduced_and_recorded() {
    let tmp = tempfile::tempdir().unwrap();
    let mut scorer = Scorer::new(tmp.path(), ScorerConfig::default()).unwrap();
    let cand = Candidate::new("c", generalizer());
    let suite = TaskSuite::new("tok", vec![Task::new("t".into(), "s".into())]);

    let single = JudgePanel::new(vec![
        Box::new(ClosureJudge::new("j1", "onlyFam", |_: &Artifact, _: &Task| 0.9))
            as Box<dyn Judge>,
        Box::new(ClosureJudge::new("j2", "onlyFam", |_: &Artifact, _: &Task| 0.85)),
    ]);
    let r = scorer.tier02_rubric(&cand, &suite, &single).unwrap();
    assert_eq!(r.families, 1);
    assert!(
        (r.veto_weight - 0.5).abs() < 1e-9,
        "where only one family is available the veto weight is reduced"
    );
    assert!((r.same_family_disagreement - 0.05).abs() < 1e-9);
    assert!(r.cross_family_disagreement.abs() < 1e-9);

    let multi = JudgePanel::new(vec![
        Box::new(ClosureJudge::new("j1", "famA", |_: &Artifact, _: &Task| 0.9)) as Box<dyn Judge>,
        Box::new(ClosureJudge::new("j2", "famB", |_: &Artifact, _: &Task| 0.85)),
    ]);
    let r = scorer.tier02_rubric(&cand, &suite, &multi).unwrap();
    assert_eq!(r.families, 2);
    assert!((r.veto_weight - 1.0).abs() < 1e-9, "multi-family: full weight");
    assert!((r.cross_family_disagreement - 0.05).abs() < 1e-9);

    // the record says so
    let texts = scorer.event_texts();
    assert!(
        texts
            .iter()
            .any(|t| t.contains("tier02") && t.contains("families=1") && t.contains("veto_weight=0.5")),
        "single-family discount is on the record"
    );
    assert!(
        texts
            .iter()
            .any(|t| t.contains("families=2") && t.contains("veto_weight=1.0")),
        "full weight with cross-family coverage"
    );
}
