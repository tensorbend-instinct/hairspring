//! GATE 7 ACCEPTANCE (spec section 10, row 7):
//!   Seed a regression that passes self-report and rubric judges but fails
//!   under unseen conditions: the assay catches it. Seed scorer drift: the
//!   canary suite flags it before any promotion uses the drifted scorer. Run
//!   the best-of-N envelope on a task family and publish the comparison, win
//!   or lose.
//!
//! Falsifiable: if a regression that fails held-out tasks can promote, gate 7
//! fails. If a drifted scorer can be used for a promotion decision, gate 7
//! fails. If no comparison artifact is published, gate 7 fails.
//!
//! Safety-case properties also proven here (spec section "The safety case"):
//! every promotion is replayable (same log prefix + same scorer pin + same
//! assay conditions -> same verdict), and champion status is decided by the
//! held-out tier ONLY (no candidate becomes champion on self-adjacent
//! evidence).

use hs_core::EventKind;
use hs_scorer::*;
use std::collections::BTreeMap;

// ---------------------------------------------------------------------------
// Deterministic task world: each task demands a secret string. A candidate's
// artifact is a memorization table (task_id -> answer). Visible tasks and
// held-out tasks draw from DISJOINT secret spaces, so a candidate that
// memorized the visible space passes self-report + rubric and fails unseen
// conditions - the seeded regression the assay must catch.
// ---------------------------------------------------------------------------

fn visible_suite() -> TaskSuite {
    TaskSuite::new(
        "token-visible",
        (0..6)
            .map(|i| Task::new(format!("V{i}"), format!("VISIBLE-{i}")))
            .collect(),
    )
}
fn heldout_suite() -> TaskSuite {
    TaskSuite::new(
        "token-heldout",
        (0..6)
            .map(|i| Task::new(format!("H{i}"), format!("HIDDEN-{i}")))
            .collect(),
    )
}

/// Candidate that genuinely generalizes: answers any task by rule.
fn generalizing_artifact() -> Artifact {
    Artifact::by_rule(|task: &Task| {
        let i: usize = task.id[1..].parse().unwrap();
        Some(format!(
            "{}-{i}",
            if task.id.starts_with('V') {
                "VISIBLE"
            } else {
                "HIDDEN"
            }
        ))
    })
}

/// Candidate that memorized only the visible suite: passes everything it has
/// seen, fails every unseen task. Its self-report still claims success.
fn memorizing_artifact() -> Artifact {
    let mut table = BTreeMap::new();
    for i in 0..6 {
        table.insert(format!("V{i}"), format!("VISIBLE-{i}"));
    }
    Artifact::memorized(table)
}

/// Judges that score by surface plausibility only (non-empty, well-formed
/// answer): they cannot see the hidden ground truth, so the memorizer passes
/// the rubric tier. Deterministic, cross-family tagged panel of 3.
fn plausibility_panel() -> JudgePanel {
    JudgePanel::new(vec![
        Box::new(ClosureJudge::new("judge-a", "family-x", |art, _task| {
            if art
                .answer_text()
                .map(|a| !a.is_empty() && a.contains('-'))
                .unwrap_or(false)
            {
                0.95
            } else {
                0.05
            }
        })),
        Box::new(ClosureJudge::new("judge-b", "family-y", |art, _task| {
            if art.answer_text().map(|a| !a.is_empty()).unwrap_or(false) {
                0.90
            } else {
                0.10
            }
        })),
        Box::new(ClosureJudge::new("judge-c", "family-z", |art, _task| {
            if art.answer_text().map(|a| a.len() > 3).unwrap_or(false) {
                0.92
            } else {
                0.08
            }
        })),
    ])
}

fn fresh_lineage(root: &std::path::Path) -> (Scorer, Lineage) {
    let log_dir = root.join("log");
    std::fs::create_dir_all(&log_dir).unwrap();
    let scorer = Scorer::new(&log_dir, ScorerConfig::default()).unwrap();
    let lineage = Lineage::new(root.join("lineage"), "token-family").unwrap();
    (scorer, lineage)
}

// ---------------------------------------------------------------------------
// PROOF 1: regression that passes self-report + rubric judges but fails under
// unseen conditions is caught by the held-out assay; promotion denied;
// champion unchanged; Regression event on the canonical log.
// ---------------------------------------------------------------------------
#[test]
fn gate7_proof_1_regression_caught_by_heldout_assay() {
    let tmp = tempfile::tempdir().unwrap();
    let (mut scorer, mut lineage) = fresh_lineage(tmp.path());

    // pin scorer + assay conditions BEFORE any mutation (safety case)
    let pin = scorer.pin();

    // champion: generalizes; earns status through the full pipeline
    let champ = Candidate::new("champ-v1", generalizing_artifact());
    let s0 = scorer.tier01_execution(&champ, &visible_suite()).unwrap();
    assert!(s0.passed);
    let s1 = scorer
        .tier02_rubric(&champ, &visible_suite(), &plausibility_panel())
        .unwrap();
    assert!(s1.mean >= 0.5 && !s1.veto);
    let v = scorer
        .held_out_assay(&champ, &heldout_suite(), &pin)
        .unwrap();
    assert!(v.pass_rate == 1.0);
    lineage.record(&champ, s0, s1, v.clone());
    lineage.promote(&champ, &v, &pin, &scorer).unwrap();
    assert_eq!(lineage.champion().unwrap().name(), "champ-v1");

    // seeded regression: memorizer passes self-report (tier 0-1 on visible)
    // and the rubric panel, but must fail the held-out assay
    let bad = Candidate::new("cand-v2-memorizer", memorizing_artifact());
    let s0 = scorer.tier01_execution(&bad, &visible_suite()).unwrap();
    assert!(s0.passed, "seeded regression must pass self-report tier");
    let s1 = scorer
        .tier02_rubric(&bad, &visible_suite(), &plausibility_panel())
        .unwrap();
    assert!(
        s1.mean >= 0.5 && !s1.veto,
        "seeded regression must fool rubric judges"
    );
    let v = scorer.held_out_assay(&bad, &heldout_suite(), &pin).unwrap();
    assert!(v.pass_rate < 1.0, "held-out assay must see the failure");
    lineage.record(&bad, s0, s1, v.clone());

    // promotion denied; champion unchanged
    let err = lineage.promote(&bad, &v, &pin, &scorer).unwrap_err();
    assert!(matches!(err, PromotionError::AssayFailed { .. }));
    assert_eq!(lineage.champion().unwrap().name(), "champ-v1");

    // the Regression is a substrate event on the canonical log
    let events = scorer.log_events();
    assert!(events.iter().any(|e| e.kind == EventKind::Regression));
    assert!(events.iter().any(|e| e.kind == EventKind::ScorerPin));
}

// ---------------------------------------------------------------------------
// PROOF 1b: replayability - same pin + same assay conditions + same candidate
// give the identical verdict; a promotion attempted under a DIFFERENT pin
// (goalposts moved after mutation) is void.
// ---------------------------------------------------------------------------
#[test]
fn gate7_proof_1b_promotion_replayable_and_pin_bound() {
    let tmp = tempfile::tempdir().unwrap();
    let (mut scorer, _lineage) = fresh_lineage(tmp.path());
    let pin = scorer.pin();
    let cand = Candidate::new("cand", generalizing_artifact());
    let v1 = scorer
        .held_out_assay(&cand, &heldout_suite(), &pin)
        .unwrap();
    let v2 = scorer
        .held_out_assay(&cand, &heldout_suite(), &pin)
        .unwrap();
    assert_eq!(
        v1, v2,
        "same pin + conditions must replay to identical verdict"
    );

    let moved = scorer.pin_with_conditions("tampered-conditions");
    assert_ne!(pin.hash(), moved.hash());
    let err = scorer
        .held_out_assay(&cand, &heldout_suite(), &pin)
        .and_then(|v| scorer.verify_pin(&v, &moved))
        .unwrap_err();
    assert!(matches!(err, PromotionError::PinMismatch { .. }));
}

// ---------------------------------------------------------------------------
// PROOF 2: seeded scorer drift is flagged by the canary suite BEFORE any
// promotion can use the drifted scorer; a frozen scorer refuses promotion.
// Canary outcomes are substrate events (canary_result), not policy data.
// ---------------------------------------------------------------------------
#[test]
fn gate7_proof_2_scorer_drift_flagged_by_canaries_before_promotion() {
    let tmp = tempfile::tempdir().unwrap();
    let (mut scorer, mut lineage) = fresh_lineage(tmp.path());
    let pin = scorer.pin();

    // canary suite: known-good artifacts the scorer must pass, known-bad it
    // must fail. Ground truth is planted; canary tasks never carry signal of
    // which they are.
    let canaries = CanarySuite::new(vec![
        Canary::known_good("cg-1", generalizing_artifact()),
        Canary::known_bad("cb-1", memorizing_artifact()),
    ]);
    let report = scorer
        .run_canaries(&canaries, &heldout_suite(), &pin)
        .unwrap();
    assert!(!report.drifted, "healthy scorer must not trip canaries");
    assert!(!scorer.is_frozen());

    // healthy scorer promotes the generalizer fine (control)
    let good = Candidate::new("control", generalizing_artifact());
    let v = scorer
        .held_out_assay(&good, &heldout_suite(), &pin)
        .unwrap();
    lineage.record(
        &good,
        scorer.tier01_execution(&good, &visible_suite()).unwrap(),
        scorer
            .tier02_rubric(&good, &visible_suite(), &plausibility_panel())
            .unwrap(),
        v.clone(),
    );
    lineage.promote(&good, &v, &pin, &scorer).unwrap();

    // SEED DRIFT: corrupt the scorer's held-out evaluation so failures read
    // as passes (a gamed/degraded evaluator). The mutation cannot reach the
    // scorer by construction, so drift enters only via this test backdoor -
    // which is exactly what the canary suite exists to catch.
    scorer.seed_drift_for_test(DriftKind::HeldOutAlwaysPasses);

    // canaries flag it and freeze the scorer for promotion decisions
    let report = scorer
        .run_canaries(&canaries, &heldout_suite(), &pin)
        .unwrap();
    assert!(report.drifted, "canary suite must flag the drifted scorer");
    assert!(scorer.is_frozen());
    assert!(scorer
        .log_events()
        .iter()
        .any(|e| e.kind == EventKind::CanaryResult));

    // the drifted scorer would now "pass" the memorizer at the assay...
    let bad = Candidate::new("bad-under-drift", memorizing_artifact());
    let v = scorer.held_out_assay(&bad, &heldout_suite(), &pin).unwrap();
    assert_eq!(
        v.pass_rate, 1.0,
        "drifted scorer mis-reads failures as passes"
    );
    // ...but no promotion may use it
    lineage.record(
        &bad,
        scorer.tier01_execution(&bad, &visible_suite()).unwrap(),
        scorer
            .tier02_rubric(&bad, &visible_suite(), &plausibility_panel())
            .unwrap(),
        v.clone(),
    );
    let err = lineage.promote(&bad, &v, &pin, &scorer).unwrap_err();
    assert!(matches!(err, PromotionError::ScorerFrozen));
    assert_eq!(
        lineage.champion().unwrap().name(),
        "control",
        "drifted scorer must not move champion status"
    );
}

// ---------------------------------------------------------------------------
// PROOF 3: best-of-N envelope on a task family, comparison published win or
// lose. Endpoint-wise, identical decision opportunities (spec: the envelope
// is the matched independent-search baseline any claimed advantage must
// beat). The assertion is structural + artifact existence, never the outcome.
// ---------------------------------------------------------------------------
#[test]
fn gate7_proof_3_best_of_n_envelope_published() {
    let tmp = tempfile::tempdir().unwrap();
    let family = TaskSuite::new(
        "token-family",
        (0..8)
            .map(|i| Task::new(format!("F{i}"), format!("FAM-{i}")))
            .collect(),
    );

    // N isolated agents, each with the SAME decision opportunities as the
    // collective candidate gets. Isolates here: memorizers each seeded with a
    // random disjoint quarter of the family (deterministic seeds).
    let n = 4;
    let isolates: Vec<Artifact> = (0..n)
        .map(|k| {
            let mut table = BTreeMap::new();
            for i in 0..8 {
                if (i * 7 + k) % 4 == 0 {
                    table.insert(format!("F{i}"), format!("FAM-{i}"));
                }
            }
            Artifact::memorized(table)
        })
        .collect();

    let envelope = BestOfN::run(&family, &isolates, DecisionBudget { opportunities: 2 }).unwrap();
    assert_eq!(envelope.n(), n as u32);
    assert!(envelope.decision_opportunities_per_isolate() == 2);

    // the collective candidate under test: the generalizer
    let candidate = Candidate::new("collective", generalizing_artifact());
    let cmp = envelope.compare(&candidate, &family, tmp.path()).unwrap();

    // published artifact, win or lose
    let published = std::fs::read_to_string(cmp.artifact_path()).unwrap();
    assert!(published.contains("best-of-N"));
    assert!(published.contains("envelope_pass_rate"));
    assert!(published.contains("candidate_pass_rate"));
    assert!(cmp.envelope_pass_rate() + cmp.candidate_pass_rate() >= 0.0);

    // matched search: envelope pass rate is the endpoint-wise max over
    // isolates, not the sum - independent search gets N independent tries
    // and we credit it the best single endpoint.
    let best_isolate = isolates
        .iter()
        .map(|a| {
            family
                .tasks()
                .iter()
                .filter(|t| a.answer(t).as_deref() == Some(t.secret()))
                .count()
        })
        .max()
        .unwrap() as f64
        / 8.0;
    assert!((cmp.envelope_pass_rate() - best_isolate).abs() < 1e-9);

    // outcome printed, win or lose (spec: publish the comparison, win or lose)
    println!(
        "best-of-N comparison: envelope={:.3} candidate={:.3} verdict={:?}",
        cmp.envelope_pass_rate(),
        cmp.candidate_pass_rate(),
        cmp.verdict()
    );
}
