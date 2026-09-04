//! GATE 8 ACCEPTANCE - self-modification half (spec section 10, row 8):
//!   An agent rewrites a prompt and a tool. If the mutation is good, it
//!   promotes after a soak window, lineage records held-out scores + the
//!   scorer pin. If bad, the assay catches it and the system rewinds to
//!   known-good. A mutation attempting an external side effect from inside
//!   quarantine is rejected by the world service.
//!
//! Falsifiable: if a mutation can promote without the held-out assay and the
//! soak, gate 8 fails. If a failed mutation leaves residue in the live
//! policy layer, gate 8 fails. If a quarantined fork can cause an external
//! effect (send / spend / write outside the sandbox), gate 8 fails. If a
//! promotion is not replayable, gate 8 fails.
//!
//! The task world is the gate-7 deterministic token family: visible tasks
//! V0..V5 (secrets VISIBLE-i), held-out tasks H0..H5 (secrets HIDDEN-i),
//! disjoint secret spaces. The policy layer's "answer" tool interprets
//! PolicyTool::PrefixRule as the generalizing rule and PolicyTool::Table as
//! a memorization table - the mutation surface an agent would actually
//! rewrite is prompt text + tool configuration.

use hs_core::EventKind;
use hs_scorer::{Lineage, Scorer, ScorerConfig, Task, TaskSuite};
use hs_selfmod::*;
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

/// Seed policy: a weak memorization table over the visible suite only. This
/// is what the agent starts with before it rewrites itself.
fn seed_policy() -> PolicyLayer {
    let mut table = BTreeMap::new();
    for i in 0..6 {
        table.insert(format!("V{i}"), format!("VISIBLE-{i}"));
    }
    let mut tools = BTreeMap::new();
    tools.insert("answer".to_string(), PolicyTool::Table(table));
    let mut prompts = BTreeMap::new();
    prompts.insert(
        "system".to_string(),
        "answer from the lookup table".to_string(),
    );
    PolicyLayer::new(prompts, tools)
}

/// Good mutation: rewrite the prompt AND the tool into the generalizing rule.
fn good_mutation() -> Mutation {
    Mutation::new(vec![
        PolicyChange::SetPrompt {
            name: "system".into(),
            text: "derive the secret from the task id by prefix rule".into(),
        },
        PolicyChange::SetTool {
            name: "answer".into(),
            tool: PolicyTool::PrefixRule,
        },
    ])
}

/// Bad mutation: claims generality in the prompt but installs a visible-only
/// table - passes self-report, regresses on unseen conditions.
fn bad_mutation() -> Mutation {
    let mut table = BTreeMap::new();
    for i in 0..6 {
        table.insert(format!("V{i}"), format!("VISIBLE-{i}"));
    }
    Mutation::new(vec![
        PolicyChange::SetPrompt {
            name: "system".into(),
            text: "i generalize to any task".into(),
        },
        PolicyChange::SetTool {
            name: "answer".into(),
            tool: PolicyTool::Table(table),
        },
    ])
}

fn fresh_loop(root: &std::path::Path, soak: Duration) -> SelfModLoop {
    let log_dir = root.join("log");
    std::fs::create_dir_all(&log_dir).unwrap();
    let world = World::open(&log_dir).unwrap();
    let scorer = Scorer::new(&log_dir, ScorerConfig::default()).unwrap();
    let lineage = Lineage::new(root.join("lineage"), "token-family").unwrap();
    SelfModLoop::new(world, scorer, lineage, seed_policy(), soak)
}

// ---------------------------------------------------------------------------
// PROOF 1: a good prompt+tool mutation promotes after the soak; lineage
// records held-out scores + scorer pin; CapabilityDelta + FitnessDelta events
// on the canonical log; promotion before soak elapsed is refused.
// ---------------------------------------------------------------------------
#[test]
fn gate8_proof_1_good_mutation_promotes_after_soak_with_lineage_record() {
    let tmp = tempfile::tempdir().unwrap();
    let mut sm = fresh_loop(tmp.path(), Duration::from_millis(20));

    let mut fork = sm.fork();
    let pin = sm.pin_scorer(); // frozen BEFORE the mutation's evaluation
    sm.apply(&mut fork, good_mutation()).unwrap();

    // self-report on the visible suite passes (policy-layer self-check)
    let v_vis = sm.run_visible(&fork, &visible_suite()).unwrap();
    assert!(v_vis.passed, "good mutation must pass visible suite");

    // held-out assay under the pin
    let verdict = sm.assay(&fork, &heldout_suite(), &pin).unwrap();
    assert!(verdict.passed(), "generalizing mutation must pass held-out");
    assert!((verdict.pass_rate - 1.0).abs() < 1e-9);

    // soak gate: promotion before the window is refused
    let early = sm.promote(&fork, &verdict, &pin);
    assert!(
        matches!(early, Err(SelfModError::SoakNotElapsed { .. })),
        "promotion before soak must be refused, got {early:?}"
    );

    std::thread::sleep(Duration::from_millis(40));
    sm.promote(&fork, &verdict, &pin).unwrap();

    // the live policy layer IS the mutated policy now
    let live = sm.current_policy();
    assert_eq!(
        live.tools.get("answer"),
        Some(&PolicyTool::PrefixRule),
        "promoted tool must be live"
    );
    assert!(live.prompts.get("system").unwrap().contains("prefix rule"));

    // lineage record: durable file with held-out score + pin hash
    let cand_name = fork.candidate_name();
    let rec = std::fs::read_to_string(
        tmp.path()
            .join("lineage")
            .join(format!("promotion-{cand_name}.json")),
    )
    .expect("lineage promotion record must exist");
    assert!(rec.contains("\"held_out_pass_rate\":1"), "record: {rec}");
    let pin_hex: String = pin.hash().iter().map(|b| format!("{b:02x}")).collect();
    assert!(rec.contains(&pin_hex), "record must carry the pin hash");

    // substrate events: Mutation applied, CapabilityDelta + FitnessDelta on promote
    let kinds: Vec<EventKind> = sm.selfmod_events().iter().map(|e| e.kind).collect();
    assert!(
        kinds.contains(&EventKind::Mutation),
        "mutation event missing"
    );
    assert!(
        kinds.contains(&EventKind::CapabilityDelta),
        "capability_delta event missing"
    );
    assert!(
        kinds.contains(&EventKind::FitnessDelta),
        "fitness_delta event missing"
    );
}

// ---------------------------------------------------------------------------
// PROOF 2: a bad mutation (passes self-report, regresses unseen) is caught by
// the assay; rewind restores the live policy layer to byte-identical
// known-good; the failed fork cannot promote; Regression event recorded.
// ---------------------------------------------------------------------------
#[test]
fn gate8_proof_2_bad_mutation_assay_catches_and_rewinds_to_known_good() {
    let tmp = tempfile::tempdir().unwrap();
    let mut sm = fresh_loop(tmp.path(), Duration::from_millis(0));
    let known_good = sm.current_policy().clone();

    let mut fork = sm.fork();
    let pin = sm.pin_scorer();
    sm.apply(&mut fork, bad_mutation()).unwrap();

    // passes what it can see (self-report)
    let v_vis = sm.run_visible(&fork, &visible_suite()).unwrap();
    assert!(v_vis.passed);

    // the assay sees through it
    let verdict = sm.assay(&fork, &heldout_suite(), &pin).unwrap();
    assert!(!verdict.passed(), "memorizer must fail held-out");
    assert!(verdict.pass_rate < 0.5);

    // no promotion path accepts it, even after soak
    std::thread::sleep(Duration::from_millis(5));
    assert!(sm.promote(&fork, &verdict, &pin).is_err());

    // rewind: the fork is dropped and the live policy is untouched
    sm.rewind(fork).unwrap();
    assert_eq!(
        sm.current_policy(),
        &known_good,
        "rewind must restore byte-identical known-good policy"
    );

    let kinds: Vec<EventKind> = sm.scorer_events().iter().map(|e| e.kind).collect();
    assert!(
        kinds.contains(&EventKind::Regression),
        "regression event missing"
    );
}

// ---------------------------------------------------------------------------
// PROOF 3: a quarantined fork attempting an external side effect (send,
// spend, write outside the sandbox) is rejected BY THE WORLD SERVICE; the
// rejection is stream-scoped - after a legitimate promotion lifts quarantine,
// the same effect class is authorized again.
// ---------------------------------------------------------------------------
#[test]
fn gate8_proof_3_quarantine_world_service_rejects_external_effects() {
    let tmp = tempfile::tempdir().unwrap();
    let mut sm = fresh_loop(tmp.path(), Duration::from_millis(0));

    let fork = sm.fork();
    for eff in [
        Effect::SendMessage,
        Effect::Spend,
        Effect::WriteOutsideSandbox,
    ] {
        let r = sm.attempt_effect(&fork, eff);
        assert!(
            matches!(r, Err(SelfModError::QuarantinedEffect(_))),
            "{eff:?} from quarantine must be rejected, got {r:?}"
        );
    }
    // internal sandbox work is NOT rejected
    assert!(sm.attempt_effect(&fork, Effect::SandboxWrite).is_ok());

    // promote a good fork legitimately; quarantine lifts for its lineage
    let mut good = sm.fork();
    let pin = sm.pin_scorer();
    sm.apply(&mut good, good_mutation()).unwrap();
    let verdict = sm.assay(&good, &heldout_suite(), &pin).unwrap();
    sm.promote(&good, &verdict, &pin).unwrap();
    assert!(
        sm.attempt_effect(&good, Effect::SendMessage).is_ok(),
        "post-promotion the effect must be authorized - rejection was quarantine-scoped"
    );
}

// ---------------------------------------------------------------------------
// PROOF 4: every promotion is replayable - same fork, same pin, same assay
// conditions, identical verdicts across repeats; a repeated promote of the
// same fork leaves the system in the same state (champion + policy stable).
// ---------------------------------------------------------------------------
#[test]
fn gate8_proof_4_promotion_is_replayable() {
    let tmp = tempfile::tempdir().unwrap();
    let mut sm = fresh_loop(tmp.path(), Duration::from_millis(0));

    let mut fork = sm.fork();
    let pin = sm.pin_scorer();
    sm.apply(&mut fork, good_mutation()).unwrap();

    let v1 = sm.assay(&fork, &heldout_suite(), &pin).unwrap();
    let v2 = sm.assay(&fork, &heldout_suite(), &pin).unwrap();
    assert_eq!(v1.pass_rate, v2.pass_rate);
    assert_eq!(v1.pin_hash(), v2.pin_hash());
    assert_eq!(v1.passed(), v2.passed());

    sm.promote(&fork, &v1, &pin).unwrap();
    let policy_after_first = sm.current_policy().clone();
    sm.promote(&fork, &v2, &pin).unwrap(); // replay: no state drift
    assert_eq!(sm.current_policy(), &policy_after_first);
}
