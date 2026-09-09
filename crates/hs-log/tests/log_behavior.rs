//! Gate-1 TDD: log behavior contract, written before the implementation.
//! Spec section 3 + gate-1 row of section 10.

use hs_core::*;
use hs_log::testing::*;
use hs_log::*;
use uuid::Uuid;

fn root() -> tempfile::TempDir {
    tempfile::tempdir().unwrap()
}
fn sid() -> Uuid {
    Uuid::from_bytes([7; 16])
}

#[test]
fn append_assigns_seq_chain_and_hash() {
    let dir = root();
    let mut w = StreamWriter::create(dir.path(), sid()).unwrap();
    let e0 = w
        .append(EventBuilder::new(EventKind::ToolCall).payload(Payload::Inline(b"a".to_vec())))
        .unwrap();
    let e1 = w
        .append(EventBuilder::new(EventKind::ToolCall).payload(Payload::Inline(b"b".to_vec())))
        .unwrap();
    assert_eq!(e0.seq, 0);
    assert_eq!(e1.seq, 1);
    assert_eq!(e0.prev_hash, [0u8; 32], "genesis prev_hash is zero");
    assert_eq!(e1.prev_hash, e0.hash);
    assert!(e0.verify_hash() && e1.verify_hash());
}

#[test]
fn clean_resume_continues_chain_without_rewrite() {
    let dir = root();
    let mut w = StreamWriter::create(dir.path(), sid()).unwrap();
    for i in 0..5u8 {
        w.append(EventBuilder::new(EventKind::ToolCall).payload(Payload::Inline(vec![i])))
            .unwrap();
    }
    let tail_hash = w.last_hash();
    drop(w);
    let before = StreamReader::open(dir.path(), sid())
        .unwrap()
        .events()
        .unwrap();

    let outcome = StreamWriter::resume(dir.path(), sid()).unwrap();
    assert_eq!(outcome.events_recovered, 5);
    assert_eq!(outcome.truncated_bytes, 0);
    let mut w = outcome.writer;
    let e5 = w.append(EventBuilder::new(EventKind::ToolCall)).unwrap();
    assert_eq!(e5.seq, 5);
    assert_eq!(e5.prev_hash, tail_hash);
    let after = StreamReader::open(dir.path(), sid())
        .unwrap()
        .events()
        .unwrap();
    assert_eq!(&after[..5], &before[..], "resume rewrote history");
}

#[test]
fn torn_tail_from_kill_is_truncated_on_resume() {
    let dir = root();
    let mut w = StreamWriter::create(dir.path(), sid()).unwrap();
    for i in 0..3u8 {
        w.append(EventBuilder::new(EventKind::ToolCall).payload(Payload::Inline(vec![i])))
            .unwrap();
    }
    let good_hash = w.last_hash();
    drop(w);
    // Simulate SIGKILL mid-write: append half a frame to the segment file.
    let seg = dir
        .path()
        .join("streams")
        .join(sid().to_string())
        .join("seg-000000.hslog");
    use std::io::Write;
    let mut f = std::fs::OpenOptions::new().append(true).open(&seg).unwrap();
    f.write_all(&[0xAA, 0xBB, 0xCC]).unwrap(); // partial length prefix
    f.sync_all().unwrap();
    drop(f);

    let outcome = StreamWriter::resume(dir.path(), sid()).unwrap();
    assert_eq!(outcome.events_recovered, 3);
    assert_eq!(outcome.truncated_bytes, 3);
    assert_eq!(outcome.writer.last_hash(), good_hash);
    verify_stream(dir.path(), sid()).unwrap();
}

#[test]
fn large_payload_is_stored_by_hash_and_reads_back() {
    let dir = root();
    let big: Vec<u8> = (0..INLINE_CAP + 100).map(|i| (i % 251) as u8).collect();
    let mut w = StreamWriter::create(dir.path(), sid()).unwrap();
    let e = w
        .append(EventBuilder::new(EventKind::Observation).payload(Payload::Inline(big.clone())))
        .unwrap();
    match &e.payload {
        Payload::BlobRef { len, .. } => assert_eq!(*len, big.len() as u64),
        other => panic!("large payload stayed inline: {other:?}"),
    }
    let r = StreamReader::open(dir.path(), sid()).unwrap();
    let back = r.events().unwrap();
    let resolved = r.resolve_payload(&back[0]).unwrap();
    assert_eq!(resolved, big);
    verify_stream(dir.path(), sid()).unwrap(); // verifies blob bytes too
}

#[test]
fn corruption_is_caught_at_exact_seq() {
    for flip_seq in [0u64, 1, 4] {
        let dir = root();
        let mut w = StreamWriter::create(dir.path(), sid()).unwrap();
        for i in 0..5u8 {
            w.append(EventBuilder::new(EventKind::ToolCall).payload(Payload::Inline(vec![i])))
                .unwrap();
        }
        drop(w);
        corrupt_event_byte(dir.path(), sid(), flip_seq, 60); // inside the event body
        let err = verify_stream(dir.path(), sid()).unwrap_err();
        assert_eq!(err.seq, flip_seq, "flip at {flip_seq}");
    }
}

#[test]
fn deleted_middle_event_breaks_the_chain() {
    let dir = root();
    let mut w = StreamWriter::create(dir.path(), sid()).unwrap();
    for i in 0..4u8 {
        w.append(EventBuilder::new(EventKind::ToolCall).payload(Payload::Inline(vec![i])))
            .unwrap();
    }
    drop(w);
    remove_event_from_segment(dir.path(), sid(), 2);
    let err = verify_stream(dir.path(), sid()).unwrap_err();
    assert!(
        matches!(
            err.kind,
            CorruptionKind::SeqGap { .. } | CorruptionKind::PrevHashMismatch { .. }
        ),
        "expected gap or chain break, got {:?}",
        err.kind
    );
}

#[test]
fn deleted_and_bitflipped_blobs_are_caught() {
    let dir = root();
    let big = vec![42u8; INLINE_CAP + 10];
    let mut w = StreamWriter::create(dir.path(), sid()).unwrap();
    let e = w
        .append(EventBuilder::new(EventKind::Observation).payload(Payload::Inline(big)))
        .unwrap();
    let Payload::BlobRef { hash, .. } = e.payload else {
        panic!()
    };
    drop(w);
    let blob = blob_path(dir.path(), &hash);
    std::fs::remove_file(&blob).unwrap();
    let err = verify_stream(dir.path(), sid()).unwrap_err();
    assert!(
        matches!(err.kind, CorruptionKind::BlobMissing { .. }),
        "{:?}",
        err.kind
    );

    let dir2 = root();
    let mut w2 = StreamWriter::create(dir2.path(), sid()).unwrap();
    let e2 = w2
        .append(
            EventBuilder::new(EventKind::Observation)
                .payload(Payload::Inline(vec![9u8; INLINE_CAP + 1])),
        )
        .unwrap();
    let Payload::BlobRef { hash: h2, .. } = e2.payload else {
        panic!()
    };
    drop(w2);
    let bp = blob_path(dir2.path(), &h2);
    let mut bytes = std::fs::read(&bp).unwrap();
    bytes[10] ^= 0xFF;
    std::fs::write(&bp, bytes).unwrap();
    let err = verify_stream(dir2.path(), sid()).unwrap_err();
    assert!(
        matches!(err.kind, CorruptionKind::BlobHashMismatch { .. }),
        "{:?}",
        err.kind
    );
}
