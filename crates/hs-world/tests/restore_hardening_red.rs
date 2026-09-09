//! RED (hostile review, 2026-09-09): two restore-path defects.
//!
//! 1. `parse_hex32` slices `s[i..i+2]` over a stepping range: an odd-length
//!    or non-ASCII snapshot id PANICS the restore instead of returning the
//!    documented Rejected error.
//! 2. The path-escape guard in `restore` is lexical
//!    (`dest.join(rel).starts_with(dest)`): a forged manifest entry like
//!    "../escape.txt" PASSES the check ("dest/../escape.txt" starts with
//!    "dest" lexically) and the file is written outside dest. The final
//!    manifest verification fails afterwards, but the escape has already
//!    happened - the guard's comment claims it prevents exactly this.
//!
//! Falsifiers: an odd-length id returns Err without panicking; a forged
//! manifest with a ".." entry returns Err AND no file appears outside dest.

use hs_world::*;

fn hex_of(b: &[u8; 32]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

#[test]
fn restore_rejects_odd_length_snapshot_id_without_panicking() {
    let tmp = tempfile::tempdir().unwrap();
    let w = World::open(tmp.path()).unwrap();
    let dest = tmp.path().join("dest");
    let r = w.restore("abc", &dest);
    assert!(r.is_err(), "odd-length snapshot id must be an error");
    let r = w.restore(&"zz".repeat(32), &dest);
    assert!(r.is_err(), "non-hex snapshot id must be an error");
}

#[test]
fn restore_rejects_manifest_paths_escaping_dest() {
    let tmp = tempfile::tempdir().unwrap();
    let log_root = tmp.path();
    let w = World::open(log_root).unwrap();
    // Forge a manifest: one file whose path escapes the restore dest.
    let content = b"escaped";
    let file_hash = hs_log::write_blob(log_root, content).unwrap();
    let manifest = SnapshotManifest {
        dirs: vec![],
        files: vec![SnapshotEntry {
            path: "../escape.txt".into(),
            hash: hex_of(&file_hash),
            len: content.len() as u64,
        }],
        symlinks: vec![],
    };
    let mbytes = serde_json::to_vec(&manifest).unwrap();
    let mhash = hs_log::write_blob(log_root, &mbytes).unwrap();
    let dest = tmp.path().join("dest");
    let r = w.restore(&hex_of(&mhash), &dest);
    assert!(r.is_err(), "escaping manifest path must be rejected");
    assert!(
        !tmp.path().join("escape.txt").exists(),
        "restore wrote a file outside dest before failing"
    );
}
