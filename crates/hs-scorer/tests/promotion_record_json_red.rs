//! RED (hostile review, 2026-09-09): `Lineage::promote` writes the durable
//! promotion record by hand-rolled `format!` JSON - a candidate or family
//! name containing a quote makes the record unparseable. The lineage
//! record is a substrate artifact; it must be real JSON.
//!
//! Falsifier: promote a candidate whose name contains a quote; the
//! promotion record must parse as JSON with the name intact.

use hs_scorer::*;

#[test]
fn promotion_record_is_valid_json_even_with_quoted_names() {
    let dir = tempfile::tempdir().unwrap();
    let lin_dir = tempfile::tempdir().unwrap();
    let mut scorer = Scorer::new(dir.path(), ScorerConfig::default()).unwrap();
    let suite = TaskSuite::new("heldout", vec![Task::new("V1".into(), "secret".into())]);
    let mut table = std::collections::BTreeMap::new();
    table.insert("V1".to_string(), "secret".to_string());
    let cand = Candidate::new("cand-\"quoted\"", Artifact::memorized(table));
    let pin = scorer.pin();
    let verdict = scorer.held_out_assay(&cand, &suite, &pin).unwrap();
    assert!(verdict.passed());
    let mut lin = Lineage::new(lin_dir.path().to_path_buf(), "fam").unwrap();
    lin.record(
        &cand,
        Tier01 {
            passed: true,
            tasks_correct: 1,
            tasks_total: 1,
        },
        Tier02 {
            mean: 1.0,
            ci_low: 1.0,
            ci_high: 1.0,
            veto: false,
            families: 0,
            veto_weight: VETO_WEIGHT_REDUCED,
            cross_family_disagreement: 0.0,
            same_family_disagreement: 0.0,
        },
        verdict.clone(),
    );
    lin.promote(&cand, &verdict, &pin, &scorer).unwrap();
    let rec = lin_dir.path().join("promotion-cand-\"quoted\".json");
    let body = std::fs::read_to_string(&rec).unwrap();
    let v: serde_json::Value = serde_json::from_str(&body)
        .expect("promotion record must be valid JSON");
    assert_eq!(v["candidate"].as_str().unwrap(), "cand-\"quoted\"");
    assert_eq!(v["family"].as_str().unwrap(), "fam");
}
