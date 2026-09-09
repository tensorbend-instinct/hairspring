//! RED-first pin for the stream registry (B8 glue): the selfmod chain's
//! streams exist but are undiscoverable - `SelfModLoop` and `Scorer`
//! each mint a fresh Uuid stream at creation and register it nowhere,
//! so no off-process surface (the TUI read views) can find the lineage,
//! scorer, or canary records the substrate already holds.
//!
//! Contract: `register_stream(log_root, role, id)` publishes the stream
//! for a role under the log root; `registered_stream(log_root, role)`
//! reads it back; an unregistered role reads as None; re-registering a
//! role replaces it (a fresh cycle owns its role). Roles are path-safe
//! names only.

#[test]
fn v1_register_and_discover_a_stream_by_role() {
    let dir = std::env::temp_dir().join(format!("stream-registry-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();

    let s = uuid::Uuid::new_v4();
    hs_log::register_stream(&dir, "scorer", s).unwrap();
    assert_eq!(hs_log::registered_stream(&dir, "scorer"), Some(s));
    assert_eq!(hs_log::registered_stream(&dir, "selfmod"), None);

    // a fresh cycle re-registers and owns the role
    let s2 = uuid::Uuid::new_v4();
    hs_log::register_stream(&dir, "scorer", s2).unwrap();
    assert_eq!(hs_log::registered_stream(&dir, "scorer"), Some(s2));

    // the registered stream opens and reads like any substrate stream
    let mut w = hs_log::StreamWriter::create(&dir, s2).unwrap();
    w.append(
        hs_core::EventBuilder::new(hs_core::EventKind::Score)
            .payload(hs_core::Payload::Inline(b"pin".to_vec())),
    )
    .unwrap();
    let reader = hs_log::StreamReader::open(&dir, s2).unwrap();
    assert_eq!(reader.events().unwrap().len(), 1);
}

#[test]
fn v2_roles_are_path_safe_names_only() {
    let dir = std::env::temp_dir().join(format!("stream-registry-roles-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let s = uuid::Uuid::new_v4();
    for bad in ["a/b", "a\\b", "..", "a b", ""] {
        assert!(
            hs_log::register_stream(&dir, bad, s).is_err(),
            "role {bad:?} must be refused"
        );
    }
}
