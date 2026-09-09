//! Empirical kill-and-restore experiment (spec v5 recovery tier B).
//! Usage: `snapshot_restore_bench` <`log_root`> <`ws_path`> [--keep]
//! Snapshots ws, copies a reference, destroys ws, restores, byte-compares,
//! prints measured snapshot/restore times. Exit 1 on any mismatch.
use std::path::{Path, PathBuf};

fn copy_tree(src: &Path, dst: &Path) {
    std::fs::create_dir_all(dst).unwrap();
    for e in std::fs::read_dir(src).unwrap() {
        let e = e.unwrap();
        let to = dst.join(e.file_name());
        let md = std::fs::symlink_metadata(e.path()).unwrap();
        if md.file_type().is_symlink() {
            let t = std::fs::read_link(e.path()).unwrap();
            std::os::unix::fs::symlink(t, &to).unwrap();
        } else if md.is_dir() {
            copy_tree(&e.path(), &to);
        } else {
            std::fs::copy(e.path(), &to).unwrap();
        }
    }
}

fn tree_diff(a: &Path, b: &Path) -> Vec<String> {
    let mut diffs = Vec::new();
    let mut stack = vec![(a.to_path_buf(), b.to_path_buf())];
    while let Some((pa, pb)) = stack.pop() {
        let mut ea: Vec<_> = std::fs::read_dir(&pa)
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .collect();
        let mut eb: Vec<_> = std::fs::read_dir(&pb)
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .collect();
        ea.sort();
        eb.sort();
        if ea != eb {
            diffs.push(format!("entries differ at {}", pa.display()));
            continue;
        }
        for name in ea {
            let fa = pa.join(&name);
            let fb = pb.join(&name);
            let mda = std::fs::symlink_metadata(&fa).unwrap();
            let mdb = std::fs::symlink_metadata(&fb).unwrap();
            if mda.file_type().is_symlink() || mdb.file_type().is_symlink() {
                if !(mda.file_type().is_symlink() && mdb.file_type().is_symlink())
                    || std::fs::read_link(&fa).unwrap() != std::fs::read_link(&fb).unwrap()
                {
                    diffs.push(format!("symlink differs: {}", fa.display()));
                }
            } else if mda.is_dir() {
                stack.push((fa, fb));
            } else if std::fs::read(&fa).unwrap() != std::fs::read(&fb).unwrap() {
                diffs.push(format!("content differs: {}", fa.display()));
            }
        }
    }
    diffs
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let log_root = PathBuf::from(&args[1]);
    let ws = PathBuf::from(&args[2]);
    let world = hs_world::World::open(&log_root).unwrap();
    let snap = world.snapshot(&ws).unwrap();
    println!(
        "SNAPSHOT id={} files={} bytes={} took_ms={}",
        snap.snapshot_id, snap.files, snap.bytes, snap.took_ms
    );
    let ref_ws = ws.with_extension("ref");
    if ref_ws.exists() {
        std::fs::remove_dir_all(&ref_ws).unwrap();
    }
    let t = std::time::Instant::now();
    copy_tree(&ws, &ref_ws);
    println!("REF_COPY took_ms={}", t.elapsed().as_millis());
    std::fs::remove_dir_all(&ws).unwrap();
    println!("DESTROYED {}", ws.display());
    let rest = world.restore(&snap.snapshot_id, &ws).unwrap();
    println!(
        "RESTORE id={} files={} bytes={} took_ms={}",
        rest.snapshot_id, rest.files, rest.bytes, rest.took_ms
    );
    let diffs = tree_diff(&ref_ws, &ws);
    if diffs.is_empty() {
        println!(
            "RESULT PASS byte-exact restore of {} files / {} bytes",
            rest.files, rest.bytes
        );
    } else {
        println!(
            "RESULT FAIL {} diffs: {:?}",
            diffs.len(),
            &diffs[..diffs.len().min(5)]
        );
        std::process::exit(1);
    }
}
