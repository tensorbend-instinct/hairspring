//! RED-first (checklist 7.5): the prefetch predictor is tuned BY
//! EVOLUTION - its knobs live on the policy layer (spec fig 5: prompts
//! and tool configs are the only mutable surface), so a `Mutation` retunes
//! the predictor through the same apply path as every other policy
//! change, booked as a substrate event.
//!
//! v1: `PolicyLayer` carries an optional `PrefetchPolicy`; payloads
//!     booked before the knob existed still deserialize
//!     (backward-compatible `#[serde(default)]`).
//! v2: a `SetPrefetch` mutation applies to the fork's policy layer.

use hs_scorer::{Lineage, Scorer, ScorerConfig};
use hs_selfmod::{Mutation, PolicyChange, PolicyLayer, PolicyTool, PrefetchPolicy, SelfModLoop};
use hs_world::World;
use std::collections::BTreeMap;
use std::time::Duration;

const KNOBS: PrefetchPolicy = PrefetchPolicy {
    min_samples: 2,
    cost_crossover_bp: 7_500,
};

#[test]
fn v1_policy_layer_carries_prefetch_policy() {
    let mut layer = PolicyLayer::new(BTreeMap::new(), BTreeMap::new());
    layer.prefetch = Some(KNOBS);
    let json = serde_json::to_string(&layer).unwrap();
    let back: PolicyLayer = serde_json::from_str(&json).unwrap();
    assert_eq!(back.prefetch, Some(KNOBS), "round trip: {json}");
    // payloads booked before the knob existed
    let legacy: PolicyLayer = serde_json::from_str(r#"{"prompts":{},"tools":{}}"#).unwrap();
    assert_eq!(legacy.prefetch, None);
}

#[test]
fn v2_setprefetch_mutation_applies_to_the_fork() {
    let tmp = tempfile::tempdir().unwrap();
    let log_dir = tmp.path().join("log");
    let world = World::open(&log_dir).unwrap();
    let scorer = Scorer::new(&log_dir, ScorerConfig::default()).unwrap();
    let lineage = Lineage::new(tmp.path().join("lineage"), "token-family").unwrap();
    let mut tools = BTreeMap::new();
    tools.insert("answer".to_string(), PolicyTool::PrefixRule);
    let mut sm = SelfModLoop::new(
        world,
        scorer,
        lineage,
        PolicyLayer::new(BTreeMap::new(), tools),
        Duration::from_millis(0),
    );
    let mut fork = sm.fork();
    let m = Mutation::new(vec![PolicyChange::SetPrefetch {
        min_samples: 2,
        cost_crossover_bp: 7_500,
    }]);
    sm.apply(&mut fork, m).unwrap();
    assert_eq!(
        fork.policy().prefetch,
        Some(KNOBS),
        "the mutation retunes the predictor on the policy layer"
    );
}
