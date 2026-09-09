//! RED-first (7.5, evolutionary half): the prefetch predictor is TUNED
//! BY EVOLUTION - a fork's `SetPrefetch` policy gets a held-out fitness
//! assay and the delta is booked on the canonical selfmod stream, the
//! same evidence path every fitness decision rides.
//!
//! v1: the assay is deterministic and DISCRIMINATES knobs on a workload
//!     the mutation never saw: on a miss-heavy recall stream the
//!     early-retiring policy loses fewer tokens than the defaults; on a
//!     repeat-heavy stream good knobs do not hurt a working predictor.
//! v2: `assay_prefetch` books a `FitnessDelta` naming the fork's own
//!     candidate - the promotion evidence path covers predictor tuning.

use hs_scorer::{Lineage, Scorer, ScorerConfig};
use hs_selfmod::{
    Mutation, PolicyChange, PolicyLayer, PolicyTool, PrefetchWorkload, SelfModLoop,
};
use hs_world::World;
use std::collections::BTreeMap;
use std::time::Duration;

fn fresh_loop(root: &std::path::Path) -> SelfModLoop {
    let log_dir = root.join("log");
    let world = World::open(&log_dir).unwrap();
    let scorer = Scorer::new(&log_dir, ScorerConfig::default()).unwrap();
    let lineage = Lineage::new(root.join("lineage"), "token-family").unwrap();
    let mut tools = BTreeMap::new();
    tools.insert("answer".to_string(), PolicyTool::PrefixRule);
    SelfModLoop::new(
        world,
        scorer,
        lineage,
        PolicyLayer::new(BTreeMap::new(), tools),
        Duration::from_millis(0),
    )
}

fn miss_heavy() -> PrefetchWorkload {
    PrefetchWorkload::new(
        "miss-heavy",
        (0..6)
            .map(|i| (format!("q{i}"), 40))
            .collect(),
    )
}

fn repeat_heavy() -> PrefetchWorkload {
    PrefetchWorkload::new(
        "repeat-heavy",
        (0..6).map(|_| ("same-q".to_string(), 200)).collect(),
    )
}

#[test]
fn v1_assay_is_deterministic_and_discriminates_knobs() {
    let tmp = tempfile::tempdir().unwrap();
    let mut sm = fresh_loop(tmp.path());

    // incumbent fork: no prefetch overlay -> the compiled-in defaults
    let incumbent = sm.fork();
    let f_inc = sm
        .assay_prefetch(&incumbent, &miss_heavy())
        .unwrap();
    let f_inc2 = sm
        .assay_prefetch(&incumbent, &miss_heavy())
        .unwrap();
    assert_eq!(
        f_inc.net_tokens, f_inc2.net_tokens,
        "deterministic: {f_inc:?} vs {f_inc2:?}"
    );

    // candidate fork: early-retiring predictor
    let mut cand = sm.fork();
    sm.apply(
        &mut cand,
        Mutation::new(vec![PolicyChange::SetPrefetch {
            min_samples: 2,
            cost_crossover_bp: 5_000,
        }]),
    )
    .unwrap();
    let f_cand = sm.assay_prefetch(&cand, &miss_heavy()).unwrap();

    // on a miss-heavy stream every prefetch is wasted; the policy that
    // retires after 2 resolutions loses fewer tokens than the default,
    // which burns 4 before it gives up
    assert_eq!(f_inc.issued, 4, "defaults issue 4 then retire: {f_inc:?}");
    assert_eq!(f_cand.issued, 2, "tuned policy retires earlier: {f_cand:?}");
    assert!(
        f_cand.net_tokens > f_inc.net_tokens,
        "evolution has a gradient: tuned {} > default {}",
        f_cand.net_tokens,
        f_inc.net_tokens
    );

    // on a repeat-heavy stream the predictor hits every time; neither
    // knob set retires and the tuned config must NOT hurt it
    let f_inc_r = sm.assay_prefetch(&incumbent, &repeat_heavy()).unwrap();
    let f_cand_r = sm.assay_prefetch(&cand, &repeat_heavy()).unwrap();
    assert_eq!(f_inc_r.hits, 5, "every later recall hits: {f_inc_r:?}");
    assert_eq!(
        f_cand_r.net_tokens, f_inc_r.net_tokens,
        "good predictor unharmed by the tuned knobs"
    );
}

#[test]
fn v2_assay_books_fitness_delta_naming_the_candidate() {
    let tmp = tempfile::tempdir().unwrap();
    let mut sm = fresh_loop(tmp.path());
    let mut cand = sm.fork();
    sm.apply(
        &mut cand,
        Mutation::new(vec![PolicyChange::SetPrefetch {
            min_samples: 2,
            cost_crossover_bp: 5_000,
        }]),
    )
    .unwrap();
    let name = cand.candidate_name().to_string();
    sm.assay_prefetch(&cand, &miss_heavy()).unwrap();

    // the canonical selfmod stream carries the booked fitness delta
    let reader = hs_log::StreamReader::open(&tmp.path().join("log"), sm.stream()).unwrap();
    let deltas: Vec<String> = reader
        .events()
        .unwrap()
        .iter()
        .filter(|e| e.kind == hs_core::EventKind::FitnessDelta)
        .filter_map(|e| reader.resolve_payload(e).ok())
        .map(|b| String::from_utf8_lossy(&b).into_owned())
        .collect();
    assert!(
        deltas
            .iter()
            .any(|d| d.contains(&format!("candidate={name}"))),
        "fitness delta names the fork's own candidate {name}: {deltas:?}"
    );
}

#[test]
fn v3_booked_normalized_channel_discriminates() {
    let tmp = tempfile::tempdir().unwrap();
    let mut sm = fresh_loop(tmp.path());

    let incumbent = sm.fork();
    let f_inc = sm.assay_prefetch(&incumbent, &miss_heavy()).unwrap();
    assert!(
        (f_inc.normalized - 0.2).abs() < 1e-9,
        "default on miss-heavy: 1 - 160/200 = 0.2, got {}",
        f_inc.normalized
    );

    let mut cand = sm.fork();
    sm.apply(
        &mut cand,
        Mutation::new(vec![PolicyChange::SetPrefetch {
            min_samples: 2,
            cost_crossover_bp: 5_000,
        }]),
    )
    .unwrap();
    let f_cand = sm.assay_prefetch(&cand, &miss_heavy()).unwrap();
    assert!(
        (f_cand.normalized - 0.6).abs() < 1e-9,
        "tuned on miss-heavy: 1 - 80/200 = 0.6, got {}",
        f_cand.normalized
    );

    let f_rep = sm.assay_prefetch(&cand, &repeat_heavy()).unwrap();
    assert_eq!(
        f_rep.normalized, 1.0,
        "a hitting predictor wastes nothing: {f_rep:?}"
    );
}
