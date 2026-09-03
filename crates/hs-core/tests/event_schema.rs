//! Gate-1 TDD: the event schema from spec section 3 is the contract.
//! These tests were written before the implementation.

use hs_core::*;

fn sample_event(kind: EventKind) -> Event {
    let mut e = Event {
        event_id: uuid::Uuid::from_bytes([1; 16]),
        stream_id: uuid::Uuid::from_bytes([2; 16]),
        seq: 41,
        ts_wall_ms: 1_700_000_000_123,
        kind,
        payload: Payload::Inline(b"hello payload".to_vec()),
        parent_event_id: Some(uuid::Uuid::from_bytes([3; 16])),
        latency_ms: 87,
        cost_usd_micros: 1_234_567, // $1.234567
        sandbox_snap_id: None,
        prev_hash: [9u8; 32],
        hash: [0u8; 32],
    };
    e.hash = e.compute_hash();
    e
}

#[test]
fn every_spec_kind_exists_and_roundtrips_through_encoding() {
    // The 18 kinds from the spec, in spec order.
    let spec_kinds = [
        EventKind::ModelCall, EventKind::ToolCall, EventKind::Observation,
        EventKind::Decision, EventKind::ContextInject, EventKind::Feedback,
        EventKind::SnapshotRef, EventKind::Proposal, EventKind::Consequence,
        EventKind::GoalUpdate, EventKind::BudgetUpdate, EventKind::Spawn,
        EventKind::Message, EventKind::Mutation, EventKind::Score,
        EventKind::ScorerPin, EventKind::CanaryResult, EventKind::Prefetch,
    ];
    assert_eq!(spec_kinds.len(), 18);
    for k in spec_kinds {
        let e = sample_event(k);
        let bytes = e.encode();
        let back = Event::decode(&bytes).expect("decode");
        assert_eq!(back, e, "kind {:?} did not roundtrip", k);
    }
}

#[test]
fn reserved_kinds_for_gates_7_and_8_are_in_kind_space() {
    // 2026-09-02 requirement: capability deltas (model/harness swaps) must log
    // as a different kind than evolved-fitness scores, and regression records
    // ("verified at event N, regressed at event M") are a tracked record type.
    // Reserved now so later gates need no schema migration.
    for k in [EventKind::CapabilityDelta, EventKind::FitnessDelta, EventKind::Regression] {
        let e = sample_event(k);
        let back = Event::decode(&e.encode()).expect("decode reserved kind");
        assert_eq!(back, e);
    }
}

#[test]
fn unknown_kind_tag_is_rejected_not_silently_mapped() {
    let e = sample_event(EventKind::ToolCall);
    let mut bytes = e.encode();
    // kind tag offset: after the 4-byte length prefixes of the two uuids etc.
    // Find it structurally: re-encode with a bogus tag by locating the tag
    // byte via the decoder's layout contract instead of guessing offsets.
    let off = hs_core::testing::kind_tag_offset(&bytes);
    bytes[off] = 0xFE;
    assert!(Event::decode(&bytes).is_err());
}

#[test]
fn encoding_is_deterministic() {
    let e = sample_event(EventKind::Score);
    assert_eq!(e.encode(), e.encode());
}

#[test]
fn hash_is_sha256_over_canonical_encoding_without_hash_field() {
    // Known-answer test built with an independent implementation: sha256 over
    // the documented field order, computed here by hand with sha2 directly.
    use sha2::{Digest, Sha256};
    let e = sample_event(EventKind::ToolCall);
    let mut h = Sha256::new();
    h.update(e.event_id.as_bytes());
    h.update(e.stream_id.as_bytes());
    h.update(e.seq.to_le_bytes());
    h.update(e.ts_wall_ms.to_le_bytes());
    h.update([e.kind.tag()]);
    // payload: tag 1 = Inline, then u32 len + bytes
    h.update([1u8]);
    h.update((13u32).to_le_bytes());
    h.update(b"hello payload");
    h.update([1u8]); // parent present
    h.update(e.parent_event_id.unwrap().as_bytes());
    h.update(e.latency_ms.to_le_bytes());
    h.update(e.cost_usd_micros.to_le_bytes());
    h.update([0u8]); // no sandbox snap
    h.update(e.prev_hash);
    let expected: [u8; 32] = h.finalize().into();
    assert_eq!(e.hash, expected, "canonical hash layout changed; this is a schema break");
    assert!(e.verify_hash());
}

#[test]
fn changing_any_field_breaks_verify() {
    let base = sample_event(EventKind::ModelCall);
    let mut e = base.clone();
    e.seq += 1; e.hash = e.compute_hash();
    e.seq -= 1; // restore field but keep stale hash -> verify must fail
    assert!(!e.verify_hash());
    let mut e2 = base.clone();
    e2.cost_usd_micros += 1;
    assert!(!e2.verify_hash());
}

#[test]
fn payload_variants_roundtrip() {
    for p in [
        Payload::None,
        Payload::Inline(vec![]),
        Payload::Inline(b"x".to_vec()),
        Payload::BlobRef { hash: [7u8; 32], len: 999_999 },
    ] {
        let mut e = sample_event(EventKind::Observation);
        e.payload = p.clone();
        e.hash = e.compute_hash();
        let back = Event::decode(&e.encode()).unwrap();
        assert_eq!(back.payload, p);
        assert!(back.verify_hash());
    }
}

#[test]
fn cost_is_decimal_micros_not_float() {
    // spec: cost_usd is decimal; floats are forbidden in the canonical record.
    let e = EventBuilder::new(EventKind::ModelCall).cost_usd_micros(1).build_part();
    assert_eq!(e.cost_usd_micros, 1);
}
