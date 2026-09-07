//! RED: term.exec timeout must kill the WHOLE command tree (observed live
//! 2026-09-07, freight-dispatch-shift trial: agent ran
//! `grep -r DISPATCH_EVENT_API / --include=* -l`; the 120s timeout killed
//! the bash wrapper but the orphaned grep held the stdout pipe open, so
//! wait_with_output blocked ~30 minutes per call while the grep churned the
//! whole container filesystem at 99% CPU). Kill the process group, not just
//! the direct child.

use std::time::Instant;

/// T1: a command whose background child outlives the killed wrapper must
/// still time out promptly - the pipe-EOF block is the bug.
#[test]
fn t1_timeout_kills_whole_process_tree() {
    let dir = tempfile::tempdir().unwrap();
    let t0 = Instant::now();
    let r = hs_loop::termexec::run(dir.path(), "sleep 30 & wait", 1);
    let elapsed = t0.elapsed();
    assert_eq!(r["timed_out"], true, "{r}");
    assert!(
        elapsed.as_secs() < 8,
        "timeout must kill the tree, not wait on the orphan's pipes: took {elapsed:?}"
    );
}

/// T2: plain command still returns output and exit code (regression).
#[test]
fn t2_normal_command_unaffected() {
    let dir = tempfile::tempdir().unwrap();
    let r = hs_loop::termexec::run(dir.path(), "echo hello; exit 3", 10);
    assert_eq!(r["timed_out"], false, "{r}");
    assert_eq!(r["exit_code"], 3, "{r}");
    assert!(r["stdout"].as_str().unwrap().contains("hello"), "{r}");
}

/// T3: plain sleeper times out promptly (regression).
#[test]
fn t3_plain_timeout_unaffected() {
    let dir = tempfile::tempdir().unwrap();
    let t0 = Instant::now();
    let r = hs_loop::termexec::run(dir.path(), "sleep 30", 1);
    assert_eq!(r["timed_out"], true, "{r}");
    assert!(t0.elapsed().as_secs() < 8);
}
