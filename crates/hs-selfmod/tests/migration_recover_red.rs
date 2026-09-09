//! RED (hostile review, 2026-09-09): `Migration::emit` hand-rolls JSON
//! with `format!` - a binding reference containing a quote makes the
//! logged body unparseable, and `recover` then silently returns None
//! (`.ok()?`), breaking the module's own "recoverable from the log alone"
//! contract. The body also drops the binding KIND: `recover` hardcodes
//! `Binding::model` for both sides, so a harness swap recovers as a model
//! binding and fencing checks against the recovered binding misjudge.
//!
//! Falsifiers: a swap whose reference contains a quote must recover with
//! the same reference; a harness swap must recover with kind Harness.

use hs_selfmod::migration::*;

#[test]
fn migration_recovers_references_with_quotes_and_real_kinds() {
    let dir = tempfile::tempdir().unwrap();
    let stream = uuid::Uuid::new_v4();
    {
        let mut m = Migration::begin(
            dir.path(),
            stream,
            Binding::harness("harness-\"v1\""),
            Binding::harness("harness-\"v2\""),
        )
        .unwrap();
        m.checkpoint().unwrap();
    }
    let r = Migration::recover(dir.path(), stream)
        .expect("an open transaction must recover from the log alone");
    assert_eq!(r.old().reference, "harness-\"v1\"");
    assert_eq!(r.new_binding().reference, "harness-\"v2\"");
    assert_eq!(r.old().kind, BindingKind::Harness, "kind must survive recovery");
    assert_eq!(r.new_binding().kind, BindingKind::Harness);
    assert_eq!(r.step(), MigrationStep::Checkpoint);
}
