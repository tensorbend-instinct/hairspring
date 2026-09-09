//! RED (hostile review, 2026-09-09): byte-offset tail slicing of arbitrary
//! UTF-8 command output PANICS when the cut splits a multi-byte char.
//! Same defect class as the hs-cli trace/dump truncation fixed earlier
//! this sweep. Sites: `termexec::run` tail (6000/3000), selfcheck tail
//! (2000), repexec tail (8192) - plus critic.rs and two bin sites fixed
//! under the same helper.
//!
//! Falsifiers: output shaped 'é'*k + 'x' puts the cut byte mid-char;
//! every entry point must return a result, not panic.

use std::sync::{Mutex, PoisonError};

static ENV_LOCK: Mutex<()> = Mutex::new(());

fn utf8_bomb(reps: usize) -> String {
    // 'é' (2 bytes) * reps + 'x' (1 byte) = 2*reps+1 bytes; cutting the
    // last 2*reps bytes starts at byte 1 - mid-'é'.
    format!(
        "i=0; while [ $i -lt {reps} ]; do printf '\\303\\251'; i=$((i+1)); done; printf x"
    )
}

#[test]
fn termexec_tail_does_not_panic_on_multibyte_boundary() {
    let dir = tempfile::tempdir().unwrap();
    let v = hs_loop::termexec::run(dir.path(), &utf8_bomb(3000), 120); // 6001 bytes > 6000 cap
    assert_eq!(v["exit_code"].as_i64(), Some(0));
    assert!(v["stdout"].as_str().unwrap().ends_with('x'));
}

#[test]
fn selfcheck_tail_does_not_panic_on_multibyte_boundary() {
    let _g = ENV_LOCK.lock().unwrap_or_else(PoisonError::into_inner);
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join(".hs")).unwrap();
    std::fs::write(
        dir.path().join(".hs/checks"),
        format!("{}; exit 1\n", utf8_bomb(1000)), // 2001 bytes > 2000 cap
    )
    .unwrap();
    unsafe { std::env::set_var("HS_SELFCHECK_DIRECT", "1") };
    let v = hs_loop::selfcheck::check(dir.path());
    unsafe { std::env::remove_var("HS_SELFCHECK_DIRECT") };
    assert_eq!(v["passed"].as_bool(), Some(false));
    assert!(v["error"].as_str().unwrap().contains("exited"));
}

#[test]
fn repexec_tail_does_not_panic_on_multibyte_boundary() {
    let dir = tempfile::tempdir().unwrap();
    let ws = dir.path();
    std::fs::write(ws.join("code.txt"), "broken\n").unwrap();
    let git = |args: &[&str]| {
        let st = std::process::Command::new("git")
            .args(args)
            .current_dir(ws)
            .env("GIT_AUTHOR_NAME", "t")
            .env("GIT_AUTHOR_EMAIL", "t@t")
            .env("GIT_COMMITTER_NAME", "t")
            .env("GIT_COMMITTER_EMAIL", "t@t")
            .status()
            .unwrap();
        assert!(st.success(), "git {args:?}");
    };
    git(&["init"]);
    git(&["add", "."]);
    git(&["commit", "-m", "base"]);
    let v = hs_loop::repexec::run_sandboxed_no_patch(ws, &utf8_bomb(4096), 120); // 8193 > 8192 cap
    assert_eq!(v["exit_code"].as_i64(), Some(0), "{v}");
    assert!(v["stdout"].as_str().unwrap().ends_with('x'));
}
