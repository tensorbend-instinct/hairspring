//! RED-first pin (B8 glue): `Scorer::new` registers its canonical stream
//! under the `scorer` role so off-process read surfaces (the TUI scorer
//! view) can discover the `score`/`scorer_pin`/`canary` records without
//! being handed the Uuid in-process.

#[test]
fn v1_scorer_stream_is_registered_for_discovery() {
    let dir = std::env::temp_dir().join(format!("scorer-registry-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let sc = hs_scorer::Scorer::new(&dir, hs_scorer::ScorerConfig::default()).unwrap();
    let registered = hs_log::registered_stream(&dir, "scorer")
        .expect("the scorer registers its stream at creation");
    assert_eq!(
        registered,
        sc.stream(),
        "the registered stream is the scorer's own canonical stream"
    );
}
