//! RED (deep-pass hostile review, editapply): the candidate-worktree edit
//! paths (`edit.apply` blocks, codex `*** Update File`) write with
//! `fs::write(joined_path)`. `safe_join` proves the path is LOGICALLY
//! inside the candidate, but a symlink committed in the base repo is
//! happily followed OUT of it: `read_to_string` leaks the outside file's
//! content into the splice, and the write CLOBBERS the symlink target on
//! the host. The harness runs as root on the bench boxes and the edit
//! path is NOT behind bwrap - this is an arbitrary-host-file write
//! driven by model tool args.
//!
//! s1: edit-apply block on a symlinked file must be REFUSED, victim intact.
//! s2: codex Update File on a symlinked file must be REFUSED, victim intact.
//! s3: edit under a symlinked DIRECTORY must be REFUSED, victim intact.

use std::process::Command;

fn mk_ws(victim: &std::path::Path) -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    let ws = dir.path();
    std::fs::write(ws.join("app.py"), "x = 1\n").unwrap();
    std::fs::write(victim, "victim-original\n").unwrap();
    std::os::unix::fs::symlink(victim, ws.join("link.txt")).unwrap();
    let git = |args: &[&str]| {
        assert!(Command::new("git")
            .args(args)
            .current_dir(ws)
            .env("GIT_AUTHOR_NAME", "t")
            .env("GIT_AUTHOR_EMAIL", "t@t")
            .env("GIT_COMMITTER_NAME", "t")
            .env("GIT_COMMITTER_EMAIL", "t@t")
            .status()
            .unwrap()
            .success());
    };
    git(&["init", "-q"]);
    git(&["add", "."]);
    git(&["commit", "-qm", "init"]);
    dir
}

fn victim_intact(victim: &std::path::Path) -> bool {
    std::fs::read_to_string(victim).ok().as_deref() == Some("victim-original\n")
}

#[test]
fn s1_edit_apply_block_refuses_symlink_path() {
    let vdir = tempfile::tempdir().unwrap();
    let victim = vdir.path().join("victim.txt");
    let d = mk_ws(&victim);
    let r = hs_loop::editapply::apply_blocks(
        d.path(),
        &[hs_loop::editapply::EditBlock {
            path: "link.txt".to_string(),
            old: "victim-original".to_string(),
            new: "MODEL-OWNED".to_string(),
        }],
    );
    assert_eq!(r["applied"], false, "symlink writes must be refused: {r}");
    let cause = r["$error"]
        .as_str()
        .or_else(|| r["error"].as_str())
        .unwrap_or_default();
    assert!(cause.contains("symlink"), "the refusal names the cause: {r}");
    assert!(
        victim_intact(&victim),
        "the out-of-candidate target was CLOBBERED through the symlink: {:?}",
        std::fs::read_to_string(&victim)
    );
}

#[test]
fn s2_codex_update_refuses_symlink_path() {
    let vdir = tempfile::tempdir().unwrap();
    let victim = vdir.path().join("victim.txt");
    let d = mk_ws(&victim);
    let patch = "*** Begin Patch\n*** Update File: link.txt\n-victim-original\n+MODEL-OWNED\n*** End Patch\n";
    let r = hs_loop::editapply::apply_codex_patch(d.path(), patch);
    assert_eq!(r["applied"], false, "symlink writes must be refused: {r}");
    assert!(
        r["$error"].as_str().unwrap_or_default().contains("symlink"),
        "the refusal names the cause: {r}"
    );
    assert!(
        victim_intact(&victim),
        "the out-of-candidate target was CLOBBERED through the symlink"
    );
}

#[test]
fn s3_edit_under_symlinked_dir_refused() {
    let vdir = tempfile::tempdir().unwrap();
    let victim = vdir.path().join("victim.txt");
    let wdir = tempfile::tempdir().unwrap();
    let ws = wdir.path();
    std::fs::create_dir_all(vdir.path().join("assets")).unwrap();
    std::fs::write(vdir.path().join("assets/inner.txt"), "victim-original2\n").unwrap();
    std::os::unix::fs::symlink(vdir.path().join("assets"), ws.join("assets")).unwrap();
    std::fs::write(ws.join("app.py"), "x = 1\n").unwrap();
    let git = |args: &[&str]| {
        assert!(Command::new("git")
            .args(args)
            .current_dir(ws)
            .env("GIT_AUTHOR_NAME", "t")
            .env("GIT_AUTHOR_EMAIL", "t@t")
            .env("GIT_COMMITTER_NAME", "t")
            .env("GIT_COMMITTER_EMAIL", "t@t")
            .status()
            .unwrap()
            .success());
    };
    git(&["init", "-q"]);
    git(&["add", "."]);
    git(&["commit", "-qm", "init"]);
    let r = hs_loop::editapply::apply_blocks(
        ws,
        &[hs_loop::editapply::EditBlock {
            path: "assets/inner.txt".to_string(),
            old: "victim-original2".to_string(),
            new: "MODEL-OWNED".to_string(),
        }],
    );
    assert_eq!(r["applied"], false, "symlinked-dir writes must be refused: {r}");
    assert_eq!(
        std::fs::read_to_string(vdir.path().join("assets/inner.txt")).unwrap(),
        "victim-original2\n",
        "the out-of-candidate target was CLOBBERED through the symlinked directory"
    );
    let _ = victim;
}
