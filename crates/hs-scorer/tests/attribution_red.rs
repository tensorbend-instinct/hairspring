//! GATE 9e (spec v5, "Capability changes are not mutations"; Zhao & Zhao
//! arXiv:2609.00546 migration semantics imported as design rules):
//! capability-vs-fitness attribution.
//!
//! "A model or harness swap can lift every score overnight with zero evolved
//! improvement. When a capability_change event sits between two assay
//! results, the delta is attributed to the swap, recorded against the new
//! binding, and excluded from the fitness slope. The improvement-cadence
//! protocol reads fitness deltas only: same substrate, same bindings,
//! evolved policy."
//!
//! Falsifiable: an assay jump across a capability_change boundary must NOT
//! appear in fitness_deltas(); it must surface in capability_attributed()
//! recorded against the NEW binding and pointing at the boundary event.
//! Deltas with no capability_change between them are fitness, all of them.

use hs_scorer::*;

fn suite4() -> TaskSuite {
    TaskSuite::new(
        "tokens",
        (0..4)
            .map(|i| Task::new(format!("T{i}"), format!("S{i}")))
            .collect(),
    )
}

fn artifact_answering(n: usize) -> Artifact {
    Artifact::by_rule(move |t: &Task| {
        let idx: usize = t.id.trim_start_matches('T').parse().unwrap();
        if idx < n {
            Some(t.secret().to_string())
        } else {
            None
        }
    })
}

#[test]
fn capability_jump_is_attributed_to_the_swap_and_excluded_from_fitness() {
    let dir = tempfile::tempdir().unwrap();
    let mut scorer = Scorer::new(dir.path(), ScorerConfig::default()).unwrap();
    let suite = suite4();

    // evolved improvement, same binding: 1/4 -> 2/4 = fitness delta +1
    scorer
        .tier01_execution(&Candidate::new("cand", artifact_answering(1)), &suite)
        .unwrap();
    scorer
        .tier01_execution(&Candidate::new("cand", artifact_answering(2)), &suite)
        .unwrap();

    // vendor upgrade: the swap is a first-class transition on the stream
    let cc = scorer.record_capability_change("glm-4.6", "vendor upgrade");

    // overnight lift 2/4 -> 4/4: straddles the capability_change boundary
    scorer
        .tier01_execution(&Candidate::new("cand", artifact_answering(4)), &suite)
        .unwrap();
    // same binding again, no movement: 4/4 -> 4/4 = fitness delta 0
    scorer
        .tier01_execution(&Candidate::new("cand", artifact_answering(4)), &suite)
        .unwrap();

    let fit = scorer.fitness_deltas();
    assert_eq!(
        fit.len(),
        2,
        "only same-binding deltas count as fitness: {fit:?}"
    );
    assert_eq!(fit[0].delta, 1, "evolved step 1/4->2/4: {fit:?}");
    assert_eq!(fit[1].delta, 0, "plateau on new binding: {fit:?}");

    let attr = scorer.capability_attributed();
    assert_eq!(attr.len(), 1, "one boundary-crossing delta: {attr:?}");
    assert_eq!(attr[0].delta, 2, "the overnight lift: {attr:?}");
    assert_eq!(
        attr[0].new_binding, "glm-4.6",
        "recorded against the NEW binding: {attr:?}"
    );
    assert_eq!(
        attr[0].boundary_event, cc.event_id,
        "points at the capability_change event: {attr:?}"
    );
}

#[test]
fn every_swap_boundary_is_excluded_even_with_repeated_swaps() {
    let dir = tempfile::tempdir().unwrap();
    let mut scorer = Scorer::new(dir.path(), ScorerConfig::default()).unwrap();
    let suite = suite4();

    scorer
        .tier01_execution(&Candidate::new("cand", artifact_answering(1)), &suite)
        .unwrap();
    scorer.record_capability_change("binding-a", "swap 1");
    scorer
        .tier01_execution(&Candidate::new("cand", artifact_answering(2)), &suite)
        .unwrap();
    // evolved on binding-a: 2/4 -> 3/4 = fitness
    scorer
        .tier01_execution(&Candidate::new("cand", artifact_answering(3)), &suite)
        .unwrap();
    scorer.record_capability_change("binding-b", "swap 2");
    scorer
        .tier01_execution(&Candidate::new("cand", artifact_answering(4)), &suite)
        .unwrap();

    let fit = scorer.fitness_deltas();
    assert_eq!(fit.len(), 1, "only the evolved step survives: {fit:?}");
    assert_eq!(fit[0].delta, 1);

    let attr = scorer.capability_attributed();
    assert_eq!(attr.len(), 2, "both swap boundaries: {attr:?}");
    assert_eq!(attr[0].new_binding, "binding-a");
    assert_eq!(attr[0].delta, 1);
    assert_eq!(attr[1].new_binding, "binding-b");
    assert_eq!(attr[1].delta, 1);
}
