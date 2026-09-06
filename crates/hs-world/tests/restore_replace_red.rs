//! restore_replace: apply a snapshot over a LIVE tree - rehydrate to a
//! staging dir, hash-verify there, then swap into place. The gate-8 async
//! verifier banks/vetoes against the audited ws state, so speculative
//! files created after the snapshot must be gone and tracked files must
//! return byte-exact.

#[test]
fn restore_replace_over_a_dirty_tree() {
    let dir = tempfile::tempdir().unwrap();
    let log = tempfile::tempdir().unwrap();
    let ws = dir.path().join("ws");
    std::fs::create_dir_all(&ws).unwrap();
    std::fs::write(ws.join("a.txt"), "alpha\n").unwrap();
    std::fs::create_dir_all(ws.join("sub")).unwrap();
    std::fs::write(ws.join("sub").join("b.txt"), "bravo\n").unwrap();

    let world = hs_world::World::open(log.path()).unwrap();
    let snap = world.snapshot(&ws).unwrap();

    // speculative work after the snapshot: edit a tracked file, add files,
    // remove one
    std::fs::write(ws.join("a.txt"), "ALPHA-CHANGED\n").unwrap();
    std::fs::write(ws.join("spec.txt"), "speculative\n").unwrap();
    std::fs::remove_file(ws.join("sub").join("b.txt")).unwrap();

    let rep = world.restore_replace(&snap.snapshot_id, &ws).unwrap();
    assert_eq!(rep.snapshot_id, snap.snapshot_id);
    assert_eq!(std::fs::read_to_string(ws.join("a.txt")).unwrap(), "alpha\n");
    assert_eq!(std::fs::read_to_string(ws.join("sub").join("b.txt")).unwrap(), "bravo\n");
    assert!(!ws.join("spec.txt").exists(), "speculative file removed by the swap");
    // no staging debris left behind
    let leftovers: Vec<_> = std::fs::read_dir(dir.path()).unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .filter(|n| n.starts_with(".hsstage-") || n.starts_with(".hsold-"))
        .collect();
    assert!(leftovers.is_empty(), "staging dirs cleaned: {leftovers:?}");
}

#[test]
fn restore_replace_unknown_snapshot_fails_loud_and_leaves_tree() {
    let dir = tempfile::tempdir().unwrap();
    let log = tempfile::tempdir().unwrap();
    let ws = dir.path().join("ws");
    std::fs::create_dir_all(&ws).unwrap();
    std::fs::write(ws.join("a.txt"), "alpha\n").unwrap();
    let world = hs_world::World::open(log.path()).unwrap();
    let bogus = "00".repeat(32);
    let r = world.restore_replace(&bogus, &ws);
    assert!(r.is_err(), "unknown snapshot id fails loud");
    assert_eq!(std::fs::read_to_string(ws.join("a.txt")).unwrap(), "alpha\n", "live tree untouched");
}
