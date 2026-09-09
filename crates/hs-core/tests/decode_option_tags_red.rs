//! RED (hostile review, 2026-09-09): the crate doc promises "unknown tags
//! are rejected", and `decode` honors that for kind and payload tags - but
//! the one-byte option tags for `parent_event_id` and `sandbox_snap_id`
//! accept ANY nonzero byte as `Some`. A corrupt or non-canonical tag byte
//! (0x02..=0xFF) silently decodes as `Some`, contradicting the strictness
//! contract the hash-chain audit story rests on.
//!
//! Falsifiers: a parent/snap tag byte of 0x02 must fail to decode.

use hs_core::*;

fn encoded_with_parent() -> Vec<u8> {
    let mut e = Event {
        event_id: uuid::Uuid::from_bytes([1; 16]),
        stream_id: uuid::Uuid::from_bytes([2; 16]),
        seq: 7,
        ts_wall_ms: 1_700_000_000_000,
        kind: EventKind::Decision,
        payload: Payload::None,
        parent_event_id: Some(uuid::Uuid::from_bytes([3; 16])),
        latency_ms: 0,
        cost_usd_micros: 0,
        sandbox_snap_id: None,
        prev_hash: [0u8; 32],
        hash: [0u8; 32],
    };
    e.hash = e.compute_hash();
    e.encode()
}

// Layout with Payload::None and a parent: 16 (event_id) + 16 (stream_id)
// + 8 (seq) + 8 (ts) + 1 (kind) + 1 (payload tag) = byte 50 is the parent
// tag; the parent uuid is bytes 51..67; latency 67..71; cost 71..79;
// byte 79 is the snap tag.
const PARENT_TAG_OFF: usize = 50;
const SNAP_TAG_OFF: usize = 79;

#[test]
fn decode_rejects_noncanonical_parent_option_tag() {
    let mut bytes = encoded_with_parent();
    assert_eq!(bytes[PARENT_TAG_OFF], 1, "test setup: parent tag present");
    bytes[PARENT_TAG_OFF] = 0x02;
    assert!(
        Event::decode(&bytes).is_err(),
        "non-canonical parent option tag 0x02 decoded as Some"
    );
}

#[test]
fn decode_rejects_noncanonical_snap_option_tag() {
    let mut bytes = encoded_with_parent();
    assert_eq!(bytes[SNAP_TAG_OFF], 0, "test setup: snap tag absent");
    bytes[SNAP_TAG_OFF] = 0x02;
    // Appending the snap uuid the tag now claims to introduce keeps the
    // length plausible; decode must still reject the tag itself.
    let mut with_body = bytes[..=SNAP_TAG_OFF].to_vec();
    with_body.extend_from_slice(&[4u8; 16]);
    with_body.extend_from_slice(&bytes[SNAP_TAG_OFF + 1..]);
    assert!(
        Event::decode(&with_body).is_err(),
        "non-canonical snap option tag 0x02 decoded as Some"
    );
}
