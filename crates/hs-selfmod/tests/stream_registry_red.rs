//! RED-first pin (B8 glue): `SelfModLoop::new` registers its stream
//! under the `selfmod` role so off-process read surfaces (the TUI
//! lineage view) can discover the `mutation`/`capability`/`fitness` records
//! without being handed the Uuid in-process.

use hs_scorer::{Lineage, Scorer, ScorerConfig};
use hs_selfmod::{PolicyLayer, PolicyTool, SelfModLoop};
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

#[test]
fn v1_selfmod_stream_is_registered_for_discovery() {
    let root = std::env::temp_dir().join(format!("selfmod-registry-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let log_dir = root.join("log");
    let world = World::open(&log_dir).unwrap();
    let scorer = Scorer::new(&log_dir, ScorerConfig::default()).unwrap();
    let lineage = Lineage::new(root.join("lineage"), "token-family").unwrap();
    let lp = SelfModLoop::new(world, scorer, lineage, seed(), Duration::from_millis(0));
    let registered = hs_log::registered_stream(&log_dir, "selfmod")
        .expect("the selfmod loop registers its stream at creation");
    assert_eq!(
        registered,
        lp.stream(),
        "the registered stream is the selfmod loop's own stream"
    );
}
