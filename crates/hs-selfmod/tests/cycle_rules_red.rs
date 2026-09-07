//! GATE 9d (spec v5; Yan et al. imports): the evolutionary loop's cycle
//! rules.
//!
//! Per-cycle balance: "each cycle closes an outstanding failure from
//! evidence state and adds one bounded new capability. Repair-only cycles
//! stall into repetitive local fixes; capability-only cycles accumulate
//! unverified surface."
//! Independent acceptance on a frozen candidate: "what gets accepted is the
//! candidate exactly as produced, not the producer's account of it" - a
//! verdict computed on candidate B must never promote fork A.
//!
//! Falsifiable: unbalanced proposals are refused BEFORE the fork exists;
//! a mismatched verdict is refused at promote.

use hs_scorer::{Artifact, Candidate, ScorerConfig, Task, TaskSuite};
use hs_scorer::{Lineage, Scorer};
use hs_selfmod::cycle::*;
use hs_selfmod::*;
use hs_world::World;
use std::collections::BTreeMap;
use std::time::Duration;

fn seed() -> PolicyLayer {
    let mut prompts = BTreeMap::new();
    prompts.insert("operator".to_string(), "seed".to_string());
    let mut tools = BTreeMap::new();
    tools.insert("answer".to_string(), PolicyTool::PrefixRule);
    PolicyLayer::new(prompts, tools)
}

fn fresh_loop(root: &std::path::Path) -> SelfModLoop {
    let log_dir = root.join("log");
    let world = World::open(&log_dir).unwrap();
    let scorer = Scorer::new(&log_dir, ScorerConfig::default()).unwrap();
    let lineage = Lineage::new(root.join("lineage"), "token-family").unwrap();
    SelfModLoop::new(world, scorer, lineage, seed(), Duration::from_millis(0))
}

fn heldout() -> TaskSuite {
    TaskSuite::new(
        "token-heldout",
        (0..3)
            .map(|i| Task::new(format!("H{i}"), format!("HIDDEN-{i}")))
            .collect(),
    )
}

#[test]
fn per_cycle_balance_rule_is_enforced_before_the_fork_exists() {
    let tmp = tempfile::tempdir().unwrap();
    let mut sm = fresh_loop(tmp.path());

    // put one open failure into evidence state
    let broken = Artifact::by_rule(|_| None);
    sm.scorer_mut()
        .tier01_execution(&Candidate::new("cand", broken), &heldout())
        .unwrap();
    let claim = sm.evidence_state().into_iter().next().unwrap();

    // repair-only: refused
    let r = sm.fork_cycle(&CycleProposal {
        closes_failure: Some(claim.claim_id),
        adds_capability: None,
        mutation: Mutation::new(vec![]),
    });
    assert!(
        matches!(r, Err(SelfModError::UnbalancedCycle(_))),
        "repair-only"
    );

    // capability-only: refused while a failure stands open
    let r = sm.fork_cycle(&CycleProposal {
        closes_failure: None,
        adds_capability: Some("bounded: add repo lint prompt".into()),
        mutation: Mutation::new(vec![]),
    });
    assert!(
        matches!(r, Err(SelfModError::UnbalancedCycle(_))),
        "capability-only"
    );

    // dangling failure ref: refused
    let r = sm.fork_cycle(&CycleProposal {
        closes_failure: Some(uuid::Uuid::new_v4()),
        adds_capability: Some("bounded: add repo lint prompt".into()),
        mutation: Mutation::new(vec![]),
    });
    assert!(
        matches!(r, Err(SelfModError::EvidenceMismatch(_))),
        "dangling ref"
    );

    // unbounded capability text: refused (one BOUNDED capability)
    let r = sm.fork_cycle(&CycleProposal {
        closes_failure: Some(claim.claim_id),
        adds_capability: Some("x".repeat(400)),
        mutation: Mutation::new(vec![]),
    });
    assert!(
        matches!(r, Err(SelfModError::UnbalancedCycle(_))),
        "unbounded"
    );

    // balanced: accepted, fork exists
    let r = sm.fork_cycle(&CycleProposal {
        closes_failure: Some(claim.claim_id),
        adds_capability: Some("bounded: add repo lint prompt".into()),
        mutation: Mutation::new(vec![]),
    });
    assert!(r.is_ok(), "balanced cycle must fork");
}

#[test]
fn promote_rejects_a_verdict_that_is_not_the_frozen_candidate() {
    let tmp = tempfile::tempdir().unwrap();
    let mut sm = fresh_loop(tmp.path());
    let pin = sm.pin_scorer();

    let fork = sm.fork();
    // a verdict computed on some OTHER candidate ("the producer's account")
    let other = Candidate::new(
        "someone-else",
        Artifact::by_rule(|t: &Task| {
            let i: usize = t.id[1..].parse().unwrap();
            Some(format!("HIDDEN-{i}"))
        }),
    );
    let verdict = sm
        .scorer_mut()
        .held_out_assay(&other, &heldout(), &pin)
        .unwrap();
    let r = sm.promote(&fork, &verdict, &pin);
    assert!(
        matches!(r, Err(SelfModError::FrozenMismatch(_))),
        "verdict for another candidate must not promote this fork"
    );
}
