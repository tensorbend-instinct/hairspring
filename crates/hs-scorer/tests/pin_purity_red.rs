//! RED (hostile review, 2026-09-09): `ScorerPin::compute` routes through
//! `hs_log::write_blob` on /tmp - a STORAGE operation with side effects
//! (junk blobs accumulate in /tmp/blobs) and, worse, an `unwrap_or([0;32])`
//! fallback: if the write fails, every pin hashes to zero and `PinMismatch`
//! verification silently passes for mismatched pins. A pin hash is sha256
//! over "version|conditions" - pure computation, no storage.
//!
//! Falsifier: computing a pin must not create the corresponding blob file
//! under /tmp/blobs.

use hs_scorer::*;

#[test]
fn pin_computation_writes_no_storage_blobs() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = Scorer::new(dir.path(), ScorerConfig::default()).unwrap();
    let conditions = format!("pin-purity-{}", uuid::Uuid::new_v4());
    let pin = s.pin_with_conditions(&conditions);
    let body = format!(
        "{}|{}",
        ScorerConfig::default().scorer_version,
        conditions
    );
    use sha2::Digest;
    let h: [u8; 32] = sha2::Sha256::digest(body.as_bytes()).into();
    assert_eq!(
        pin.hash(),
        h,
        "pin hash must be sha256(version|conditions)"
    );
    let hex: String = h.iter().map(|b| format!("{b:02x}")).collect();
    let blob = std::path::Path::new("/tmp")
        .join("blobs")
        .join(&hex[0..2])
        .join(&hex[2..4])
        .join(&hex);
    assert!(
        !blob.exists(),
        "pin computation wrote a storage blob at {}",
        blob.display()
    );
}
