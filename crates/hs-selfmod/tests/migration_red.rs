//! GATE 9b (spec v5, Zhao & Zhao imports): substrate swaps are TRANSACTIONS,
//! and continuation authority is FENCED.
//!
//! "Replacing the model, the executor build, or the host runs quiesce,
//! checkpoint, validate, bind, rehydrate, resume, with a single promotion
//! point. The old variant is fenced the moment the new one binds; a failed
//! transaction leaves the old variant in authority. Every step lands in the
//! log as a capability_change event."
//! "At most one executor variant holds continuation authority over a stream
//! at a time... Two live executors acting as the same run is a fencing
//! violation, not a race to tolerate."
//!
//! Seam test through the REAL log: events are read back from the canonical
//! stream; recovery reconstructs transaction + authority state from the log
//! alone (memory is a read path over the log).

use hs_core::EventKind;
use hs_log::StreamReader;
use hs_selfmod::migration::*;

fn model(r: &str) -> Binding {
    Binding::model(r)
}

/// Read every capability_change payload off the stream, in order.
fn cc_events(root: &std::path::Path, stream: uuid::Uuid) -> Vec<String> {
    StreamReader::open(root, stream)
        .and_then(|r| r.events())
        .unwrap_or_default()
        .into_iter()
        .filter(|e| e.kind == EventKind::CapabilityChange)
        .map(|e| match e.payload {
            hs_core::Payload::Inline(b) => String::from_utf8_lossy(&b).into_owned(),
            _ => String::new(),
        })
        .collect()
}

#[test]
fn happy_path_swap_is_a_logged_transaction_with_single_promotion_point() {
    let dir = tempfile::tempdir().unwrap();
    let stream = uuid::Uuid::new_v4();
    let mut auth = ContinuityAuthority::new();
    auth.claim(stream, model("glm-5.3")).unwrap();

    let mut txn = Migration::begin(dir.path(), stream, model("glm-5.3"), model("glm-5.4")).unwrap();
    txn.checkpoint().unwrap();
    txn.validate(|| true).unwrap();
    // the new binding must NOT hold authority before the promotion point
    assert!(!auth.holds(stream, &model("glm-5.4")));
    txn.bind(&mut auth).unwrap();
    txn.rehydrate().unwrap();
    txn.resume().unwrap();

    // authority moved exactly once, at bind
    assert!(auth.holds(stream, &model("glm-5.4")));
    assert!(!auth.holds(stream, &model("glm-5.3")));
    // the old variant is fenced: acting as it is a violation, not a race
    assert!(matches!(
        auth.assert_holder(stream, &model("glm-5.3")),
        Err(MigrationError::FencingViolation(_))
    ));

    // every protocol step is on the canonical log, in order, with both refs
    let steps: Vec<String> = cc_events(dir.path(), stream)
        .iter()
        .map(|p| {
            let v: serde_json::Value = serde_json::from_str(p).unwrap();
            assert_eq!(v["old_binding"], "glm-5.3");
            assert_eq!(v["new_binding"], "glm-5.4");
            v["step"].as_str().unwrap().to_string()
        })
        .collect();
    assert_eq!(
        steps,
        ["quiesce", "checkpoint", "validate", "bind", "rehydrate", "resume"]
    );
}

#[test]
fn failed_validation_leaves_old_variant_in_authority() {
    let dir = tempfile::tempdir().unwrap();
    let stream = uuid::Uuid::new_v4();
    let mut auth = ContinuityAuthority::new();
    auth.claim(stream, model("glm-5.3")).unwrap();

    let mut txn = Migration::begin(dir.path(), stream, model("glm-5.3"), model("glm-5.4")).unwrap();
    txn.checkpoint().unwrap();
    let err = txn.validate(|| false).unwrap_err();
    assert!(matches!(err, MigrationError::ValidationFailed(_)));
    txn.abort(&mut auth).unwrap();

    // old variant never lost authority; the new one never gained it
    assert!(auth.holds(stream, &model("glm-5.3")));
    assert!(!auth.holds(stream, &model("glm-5.4")));

    // the abort is on the log - a swap is never invisible to the scorer
    let steps: Vec<String> = cc_events(dir.path(), stream)
        .iter()
        .map(|p| serde_json::from_str::<serde_json::Value>(p).unwrap()["step"]
            .as_str().unwrap().to_string())
        .collect();
    assert_eq!(steps, ["quiesce", "checkpoint", "validate_failed", "abort"]);
}

#[test]
fn a_second_claimant_on_a_live_stream_is_a_fencing_violation() {
    let mut auth = ContinuityAuthority::new();
    let stream = uuid::Uuid::new_v4();
    auth.claim(stream, model("glm-5.3")).unwrap();
    // a fork under assay never holds authority; a second live claimant neither
    assert!(matches!(
        auth.claim(stream, model("glm-5.4")),
        Err(MigrationError::FencingViolation(_))
    ));
    // re-claim by the SAME holder is idempotent, not a violation
    auth.claim(stream, model("glm-5.3")).unwrap();
}

#[test]
fn recovery_rebuilds_transaction_and_authority_from_the_log_alone() {
    let dir = tempfile::tempdir().unwrap();
    let stream = uuid::Uuid::new_v4();
    let mut auth = ContinuityAuthority::new();
    auth.claim(stream, model("glm-5.3")).unwrap();

    let mut txn = Migration::begin(dir.path(), stream, model("glm-5.3"), model("glm-5.4")).unwrap();
    txn.checkpoint().unwrap();
    txn.validate(|| true).unwrap();
    txn.bind(&mut auth).unwrap();
    // "process dies" here - no flush, no checkpoint file, only the log
    drop(txn);

    let recovered = Migration::recover(dir.path(), stream).expect("an in-flight txn must recover");
    assert_eq!(recovered.old(), &model("glm-5.3"));
    assert_eq!(recovered.new(), &model("glm-5.4"));
    assert_eq!(recovered.step(), MigrationStep::Bind);
    // recovery resumes the protocol where the log left off
    let mut auth2 = ContinuityAuthority::new();
    let mut txn = recovered;
    txn.rehydrate().unwrap();
    txn.resume_with(&mut auth2).unwrap();
    let steps: Vec<String> = cc_events(dir.path(), stream)
        .iter()
        .map(|p| serde_json::from_str::<serde_json::Value>(p).unwrap()["step"]
            .as_str().unwrap().to_string())
        .collect();
    assert_eq!(
        steps,
        ["quiesce", "checkpoint", "validate", "bind", "rehydrate", "resume"]
    );
}
