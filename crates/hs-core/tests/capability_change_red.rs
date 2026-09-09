//! GATE 9a (spec v5): the event schema carries a `capability_change` kind for
//! model/harness/executor swap transactions ("The kinds are reserved in the
//! schema from gate 1, so swap transactions and evidence bookkeeping need no
//! later schema migration"; v5 event schema: "`capability_change`, #
//! model/harness/executor swap transaction: old and new binding refs +
//! protocol step").
//!
//! This is NOT the gate-8 CapabilityDelta/FitnessDelta pair - those are
//! scorer-read deltas on policy-layer promotions. A swap is a substrate
//! binding change (Zhao & Zhao migration semantics), a different claim type:
//! "a vendor upgrade can never masquerade as evolved improvement."
//!
//! Falsifiable: the kind must exist at a stable tag and roundtrip through
//! the wire encoding like every other kind.

use hs_core::*;

#[test]
fn capability_change_kind_exists_at_a_stable_tag_and_roundtrips() {
    let k = EventKind::CapabilityChange;
    // tag is stable once assigned (append-only log compatibility)
    assert_eq!(k.tag(), 67);
    assert_eq!(EventKind::from_tag(67).unwrap(), k);
    assert_ne!(k, EventKind::CapabilityDelta, "swap != scorer delta");
    assert_ne!(k, EventKind::Mutation, "swap != policy self-modification");

    // full event roundtrip through the same encode/decode path gate 1 proved
    let mut e = Event {
        event_id: uuid::Uuid::from_bytes([7; 16]),
        stream_id: uuid::Uuid::from_bytes([8; 16]),
        seq: 3,
        ts_wall_ms: 1_700_100_000_000,
        kind: k,
        payload: Payload::Inline(
            br#"{"old_binding":"glm-5.3","new_binding":"glm-5.4","step":"bind"}"#.to_vec(),
        ),
        parent_event_id: None,
        latency_ms: 0,
        cost_usd_micros: 0,
        sandbox_snap_id: None,
        prev_hash: [0u8; 32],
        hash: [0u8; 32],
    };
    e.hash = e.compute_hash();
    let bytes = e.encode();
    let back = Event::decode(&bytes).unwrap();
    assert_eq!(back.kind, EventKind::CapabilityChange);
    assert_eq!(back, e);
}
