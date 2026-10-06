//! RED-first pin for B8b: the TUI read views over the selfmod chain's
//! REGISTERED substrate streams (checklist 1.3 "the TUI cannot
//! surface/query scorer state (calibration, canary, drift) at all",
//! 1.5 "no way to see/trigger evolution; no lineage view").
//!
//! Contract: `hs_loop::tui_views::scorer_view(log_root)` and
//! `selfmod_view(log_root)` discover the streams through the B8a role
//! registry and project the events into TYPED view state for the
//! overlays. A log root with no registered stream yields None - the
//! honest "no selfmod cycle this session" state, never a fabricated
//! panel.

use hs_scorer::{
    Artifact, Canary, CanarySuite, Candidate, Lineage, Scorer, ScorerConfig, Task, TaskSuite,
};
use hs_selfmod::{PolicyChange, PolicyLayer, PolicyTool, SelfModLoop};
use hs_world::World;
use std::collections::BTreeMap;
use std::time::Duration;

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
fn memorizing_artifact() -> Artifact {
    let mut table = BTreeMap::new();
    for i in 0..6 {
        table.insert(format!("V{i}"), format!("VISIBLE-{i}"));
    }
    Artifact::memorized(table)
}
fn seed() -> PolicyLayer {
    let mut prompts = BTreeMap::new();
    prompts.insert("operator".to_string(), "seed".to_string());
    let mut tools = BTreeMap::new();
    tools.insert("answer".to_string(), PolicyTool::PrefixRule);
    PolicyLayer::new(prompts, tools)
}

#[test]
fn v1_scorer_view_projects_pins_scores_canaries_from_the_registered_stream() {
    let dir = std::env::temp_dir().join(format!("tui-scorer-view-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let mut scorer = Scorer::new(&dir, ScorerConfig::default()).unwrap();
    let pin = scorer.pin();
    let good = Candidate::new("control", generalizing_artifact());
    scorer.tier01_execution(&good, &visible_suite()).unwrap();
    let canaries = CanarySuite::new(vec![
        Canary::known_good("cg-1", generalizing_artifact()),
        Canary::known_bad("cb-1", memorizing_artifact()),
    ]);
    scorer.run_canaries(&canaries, &heldout_suite(), &pin).unwrap();

    let view = hs_loop::tui_views::scorer_view(&dir)
        .expect("the registered scorer stream projects into view state");
    assert_eq!(view.pins.len(), 1, "one scorer_pin: {view:?}");
    assert_eq!(view.pins[0].version, "hs-scorer-0.1.0");
    assert_eq!(view.pins[0].conditions, "token-family-heldout-v1");

    let t01 = view
        .scores
        .iter()
        .find(|s| s.candidate == "control")
        .expect("the tier01 score for the control candidate: {view:?}");
    assert!(t01.passed);
    assert_eq!((t01.correct, t01.total), (6, 6));

    assert_eq!(view.canaries.len(), 2, "both canaries booked: {view:?}");
    let cg = view.canaries.iter().find(|c| c.id == "cg-1").unwrap();
    assert!(cg.ground_truth_good && cg.scorer_said_good && !cg.error);
    let cb = view.canaries.iter().find(|c| c.id == "cb-1").unwrap();
    assert!(!cb.ground_truth_good && !cb.scorer_said_good && !cb.error);
}

#[test]
fn v2_selfmod_view_projects_mutations_from_the_registered_stream() {
    let root = std::env::temp_dir().join(format!("tui-selfmod-view-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let log_dir = root.join("log");
    let world = World::open(&log_dir).unwrap();
    let scorer = Scorer::new(&log_dir, ScorerConfig::default()).unwrap();
    let lineage = Lineage::new(root.join("lineage"), "token-family").unwrap();
    let mut lp = SelfModLoop::new(world, scorer, lineage, seed(), Duration::from_millis(0));

    let mut fork = lp.fork();
    let m = hs_selfmod::Mutation::new(vec![PolicyChange::SetPrompt {
        name: "operator".to_string(),
        text: "mutated".to_string(),
    }]);
    lp.apply(&mut fork, m).unwrap();

    let view = hs_loop::tui_views::selfmod_view(&log_dir)
        .expect("the registered selfmod stream projects into view state");
    assert_eq!(view.mutations.len(), 1, "one mutation booked: {view:?}");
    assert_eq!(view.mutations[0].changes, 1);
    assert_eq!(view.mutations[0].fork, fork.stream());
}

#[test]
fn v4_evidence_view_projects_the_claim_record_from_the_registered_stream() {
    // A verified-then-failed subject is a REGRESSION pointing at both
    // events; a re-verification supersedes the regression and opens a
    // fresh verified claim - the scorer's own GATE 9c fold, readable
    // from the TUI's side (checklist B8 evidence half).
    let dir = std::env::temp_dir().join(format!("tui-evidence-view-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let mut scorer = Scorer::new(&dir, ScorerConfig::default()).unwrap();
    scorer
        .tier01_execution(&Candidate::new("cand-ev", generalizing_artifact()), &visible_suite())
        .unwrap();
    scorer
        .tier01_execution(&Candidate::new("cand-ev", Artifact::by_rule(|_: &Task| None)), &visible_suite())
        .unwrap();
    scorer
        .tier01_execution(&Candidate::new("cand-ev", generalizing_artifact()), &visible_suite())
        .unwrap();

    let view = hs_loop::tui_views::evidence_view(&dir)
        .expect("the registered scorer stream projects an evidence view");
    let reg = view
        .claims
        .iter()
        .find(|c| c.kind == hs_loop::tui_views::EvidenceClaimKind::Regression)
        .expect("the regressed claim: {view:?}");
    assert!(reg.verified_at.is_some() && reg.regressed_at.is_some());
    assert_eq!(
        reg.status,
        hs_loop::tui_views::EvidenceClaimStatus::Superseded,
        "re-verification supersedes the regression, the record is kept: {view:?}"
    );
    let fresh = view
        .claims
        .iter()
        .find(|c| c.kind == hs_loop::tui_views::EvidenceClaimKind::Verified)
        .expect("the fresh verified claim after re-repair: {view:?}");
    assert_eq!(
        fresh.status,
        hs_loop::tui_views::EvidenceClaimStatus::Open
    );
    assert_ne!(
        fresh.verified_at,
        reg.verified_at,
        "the fresh claim points at the new verification event"
    );
}

#[test]
fn v5_unfoldable_drift_regressions_surface_as_unresolved_lines() {
    // The scorer's tier-3 drift path books
    // "regression candidate=X suite=S pass_rate=F" - no subject/uuid
    // refs, so the GATE 9c claim fold cannot subsume it. The view
    // shows the raw line under an explicit heading: what the log
    // verifiably holds, never silently dropped, never reshaped.
    let ev1 = uuid::Uuid::parse_str("11111111-1111-1111-1111-111111111111").unwrap();
    let ev2 = uuid::Uuid::parse_str("22222222-2222-2222-2222-222222222222").unwrap();
    let events = vec![
        (
            hs_core::EventKind::Score,
            ev1,
            "tier01 candidate=cap-7 suite=token-visible passed=true 6/6".to_string(),
        ),
        (
            hs_core::EventKind::Regression,
            ev2,
            "regression candidate=cap-7 suite=token-heldout pass_rate=0.500".to_string(),
        ),
    ];
    let view = hs_loop::tui_views::fold_evidence(&events);
    assert_eq!(
        view.claims
            .iter()
            .filter(|c| c.kind == hs_loop::tui_views::EvidenceClaimKind::Verified)
            .count(),
        1,
        "the score folded into a verified claim: {view:?}"
    );
    assert_eq!(view.claims.len(), 1, "the drift line did not fold: {view:?}");
    assert_eq!(view.unresolved_regressions.len(), 1, "and is surfaced raw");
    assert!(
        view.unresolved_regressions[0].contains("pass_rate=0.500"),
        "verbatim body: {view:?}"
    );
}

#[test]
fn v3_no_registered_stream_is_an_honest_none() {
    let dir = std::env::temp_dir().join(format!("tui-views-empty-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    assert!(hs_loop::tui_views::scorer_view(&dir).is_none());
    assert!(hs_loop::tui_views::selfmod_view(&dir).is_none());
    assert!(hs_loop::tui_views::evidence_view(&dir).is_none());
}
