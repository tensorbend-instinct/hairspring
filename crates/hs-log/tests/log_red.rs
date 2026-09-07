#[test]
fn write_blobs_bulk_roundtrips_and_dedups() {
    let dir = tempfile::tempdir().unwrap();
    let items: Vec<Vec<u8>> = (0..500).map(|i| vec![(i % 251) as u8; i + 1]).collect();
    let refs: Vec<&[u8]> = items.iter().map(|v| v.as_slice()).collect();
    let hashes = hs_log::write_blobs_bulk(dir.path(), &refs).unwrap();
    assert_eq!(hashes.len(), 500);
    for (h, data) in hashes.iter().zip(items.iter()) {
        assert_eq!(&hs_log::read_blob(dir.path(), h).unwrap(), data);
    }
    // dedup: writing the same set again returns the same hashes, no error
    let hashes2 = hs_log::write_blobs_bulk(dir.path(), &refs).unwrap();
    assert_eq!(hashes, hashes2);
    // empty batch is valid
    assert!(hs_log::write_blobs_bulk(dir.path(), &[])
        .unwrap()
        .is_empty());
}
