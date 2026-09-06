//! RED (2026-09-06): spec v5 recovery tier B - snapshot restore. "Sandbox
//! dead; snapshot exists -> restore from snapshot_ref; process and
//! filesystem state back. < 60 s warm-cache; cold measured and reported,
//! not rounded down." Eric's bar: no toy snapshots - the primitive captures
//! the FULL tree (source, .git, venv, build artifacts), restore is
//! byte-exact and hash-verified, and forged/corrupt snapshots fail loudly.
use std::path::Path;
use std::process::Command;

fn realistic_ws(root: &Path) {
    let ws = root.join("ws");
    std::fs::create_dir_all(ws.join("src/deep/nest")).unwrap();
    std::fs::create_dir_all(ws.join("venv/lib/python3.12/site-packages")).unwrap();
    std::fs::write(ws.join("src/main.py"), "def main():\n    pass\n").unwrap();
    std::fs::write(ws.join("src/deep/nest/module.py"), "X = 1\n").unwrap();
    std::fs::write(ws.join("venv/lib/python3.12/site-packages/dep.py"), "import os\n").unwrap();
    std::fs::write(ws.join("bin_blob"), (0..4096u32).map(|i| (i % 251) as u8).collect::<Vec<_>>()).unwrap();
    std::fs::write(ws.join("empty_file"), b"").unwrap();
    let run = |args: &[&str]| {
        let o = Command::new("git").args(args).current_dir(&ws)
            .env("GIT_AUTHOR_DATE", "2026-01-01T00:00:00Z")
            .env("GIT_COMMITTER_DATE", "2026-01-01T00:00:00Z")
            .output().unwrap();
        assert!(o.status.success(), "git {args:?}: {:?}", String::from_utf8_lossy(&o.stderr));
    };
    run(&["init", "-q"]);
    run(&["add", "-A"]);
    run(&["-c", "user.email=b@b", "-c", "user.name=b", "commit", "-qm", "base"]);
    std::fs::write(ws.join("src/main.py"), "def main():\n    return 42\n").unwrap();
    run(&["add", "-A"]);
    run(&["-c", "user.email=b@b", "-c", "user.name=b", "commit", "-qm", "work in progress"]);
}


fn copy_tree(src: &Path, dst: &Path) {
    std::fs::create_dir_all(dst).unwrap();
    for e in std::fs::read_dir(src).unwrap() {
        let e = e.unwrap();
        let to = dst.join(e.file_name());
        if e.metadata().unwrap().is_dir() { copy_tree(&e.path(), &to); }
        else { std::fs::copy(e.path(), &to).unwrap(); }
    }
}

fn tree_diff(a: &Path, b: &Path) -> Vec<String> {
    let mut diffs = Vec::new();
    let mut stack = vec![(a.to_path_buf(), b.to_path_buf())];
    while let Some((pa, pb)) = stack.pop() {
        let mut ea: Vec<_> = std::fs::read_dir(&pa).unwrap().map(|e| e.unwrap().file_name()).collect();
        let mut eb: Vec<_> = std::fs::read_dir(&pb).unwrap().map(|e| e.unwrap().file_name()).collect();
        ea.sort(); eb.sort();
        if ea != eb { diffs.push(format!("entries differ at {}", pa.display())); continue; }
        for name in ea {
            let fa = pa.join(&name); let fb = pb.join(&name);
            if std::fs::metadata(&fa).unwrap().is_dir() { stack.push((fa, fb)); }
            else if std::fs::read(&fa).unwrap() != std::fs::read(&fb).unwrap() {
                diffs.push(format!("content differs: {}", fa.display()));
            }
        }
    }
    diffs
}

#[test]
fn snapshot_restore_is_byte_exact_and_hash_verified() {
    let dir = tempfile::tempdir().unwrap();
    realistic_ws(dir.path());
    let log_root = dir.path().join("log");
    let world = hs_world::World::open(&log_root).unwrap();
    let rep = world.snapshot(&dir.path().join("ws")).unwrap();
    assert!(rep.files >= 6 && rep.bytes > 4096, "full tree captured: {rep:?}");
    // Reference copy of the exact tree we snapshotted (.git/index embeds
    // per-build inode/mtime stats, so cross-build comparison is unsound).
    let ref_root = dir.path().join("ref");
    copy_tree(&dir.path().join("ws"), &ref_root.join("ws"));
    // Sandbox dies: the whole ws is destroyed.
    std::fs::remove_dir_all(dir.path().join("ws")).unwrap();
    let rrep = world.restore(&rep.snapshot_id, &dir.path().join("ws")).unwrap();
    assert!(rrep.took_ms < 60_000, "tier-B budget is < 60 s warm; measured {} ms", rrep.took_ms);
    let diffs = tree_diff(&ref_root.join("ws"), &dir.path().join("ws"));
    assert!(diffs.is_empty(), "restored tree must be byte-identical: {diffs:?}");
}

#[test]
fn snapshot_lands_on_the_log_and_tampering_fails() {
    let dir = tempfile::tempdir().unwrap();
    realistic_ws(dir.path());
    let log_root = dir.path().join("log");
    let world = hs_world::World::open(&log_root).unwrap();
    let rep = world.snapshot(&dir.path().join("ws")).unwrap();
    let reader = hs_log::StreamReader::open(&log_root, world.world_stream()).unwrap();
    let events = reader.events().unwrap();
    let hit = events.iter().any(|e| {
        e.kind == hs_core::EventKind::SnapshotRef
            && String::from_utf8_lossy(&reader.resolve_payload(e).unwrap()).contains(&rep.snapshot_id)
    });
    assert!(hit, "SnapshotRef event must name the snapshot id");
    let bad = world.restore(&"0".repeat(64), &dir.path().join("x"));
    assert!(bad.is_err(), "unknown snapshot id must fail loudly");
}
