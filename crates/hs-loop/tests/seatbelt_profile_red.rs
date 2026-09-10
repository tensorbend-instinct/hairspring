//! Seatbelt confinement, macOS backend (2026-09-10): Eric ruled missions
//! are first-class on macOS, confined by the strongest available native
//! mechanism (kernel Seatbelt via sandbox-exec - the mechanism Bazel,
//! Homebrew, and Claude Code rely on, functional on macOS 15). These
//! tests pin the profile shape from Linux and run the hostile behavioral
//! suite on BOTH platforms - here against the bwrap backend, on the
//! install-gate macOS legs against the seatbelt backend.

use hs_loop::termexec;
use std::path::Path;

static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

#[test]
fn p1_profile_denies_all_writes_then_allows_root_and_tmp() {
    let p = termexec::seatbelt_profile(Some(Path::new("/work/my root")));
    let deny = p.find("(deny file-write*").expect("deny-all-writes rule");
    let allow = p.find("(allow file-write*").expect("allow rule");
    assert!(
        deny < allow,
        "the deny must precede the allows (Seatbelt: later rules win): {p}"
    );
    for needle in [
        "(allow default)",
        "(subpath \"/work/my root\")",
        "(subpath \"/private/tmp\")",
        "(subpath \"/private/var/folders\")",
        "(literal \"/dev/null\")",
    ] {
        assert!(p.contains(needle), "profile carries {needle}: {p}");
    }
    assert!(
        !p.contains("deny network"),
        "the networking-enabled ruling holds (terminal-bench contract): {p}"
    );
}

#[test]
fn p2_verifier_profile_omits_the_root() {
    let p = termexec::seatbelt_profile(None);
    assert!(
        !p.contains("/work"),
        "the verifier gets NO writable root - task files read-only by \
         mechanism (the uid-nobody lever on Linux): {p}"
    );
    assert!(p.contains("/private/tmp"), "scratch stays writable: {p}");
}

#[test]
fn p3_profile_escapes_sbpl_string_metachars() {
    let p = termexec::seatbelt_profile(Some(Path::new("/weird\"quo\\te")));
    assert!(
        p.contains("/weird\\\"quo\\\\te"),
        "a quote/backslash in the root cannot break out of the literal: {p}"
    );
}

#[test]
fn b1_write_inside_root_ok_outside_refused() {
    let _g = ENV_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().canonicalize().unwrap();
    let outside = root.parent().unwrap().join("hs-escape-probe");
    let _ = std::fs::remove_file(&outside);
    unsafe { std::env::set_var("HS_PROJECT_ROOT", &root) };
    let ok = termexec::run(&root, "echo ok > inside.txt && cat inside.txt", 10);
    assert_eq!(ok["stdout"].as_str().unwrap_or("").trim(), "ok");
    // /usr is read-only on both backends (ro-bind / seatbelt deny); a
    // parent-of-root write lands in scratch (/tmp tmpfs on Linux), so
    // /usr is the honest hostile target. Root under /tmp would make
    // ".." resolve into the sandbox's own scratch, proving nothing.
    let esc = termexec::run(
        &root,
        "touch /usr/hs-escape-probe 2>/dev/null && echo WROTE || echo REFUSED; \
         touch /etc/hs-escape-probe 2>/dev/null && echo WROTE2 || echo REFUSED2",
        10,
    );
    unsafe { std::env::remove_var("HS_PROJECT_ROOT") };
    let out = esc["stdout"].as_str().unwrap_or("");
    assert!(
        out.contains("REFUSED") && out.contains("REFUSED2"),
        "writes outside the root are refused on BOTH backends: {esc}"
    );
    assert!(
        !outside.exists() && !std::path::Path::new("/usr/hs-escape-probe").exists(),
        "nothing landed outside the project root"
    );
}

#[test]
fn b2_environment_is_scrubbed() {
    let _g = ENV_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().canonicalize().unwrap();
    unsafe {
        std::env::set_var("HS_PROJECT_ROOT", &root);
        std::env::set_var("HS_TEST_LEAK", "hunter2");
    }
    let o = termexec::run(&root, "echo PATH=$PATH; echo LEAK=${HS_TEST_LEAK-unset}", 10);
    unsafe {
        std::env::remove_var("HS_PROJECT_ROOT");
        std::env::remove_var("HS_TEST_LEAK");
    }
    let out = o["stdout"].as_str().unwrap_or("");
    assert!(out.contains("PATH=/usr/bin:/bin"), "minimal PATH: {out}");
    assert!(out.contains("LEAK=unset"), "no harness env leaks in: {out}");
}

#[test]
fn b3_verifier_cannot_write_task_files_but_keeps_scratch() {
    let _g = ENV_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().canonicalize().unwrap();
    std::fs::write(root.join("task.txt"), "graded artifact").unwrap();
    unsafe { std::env::set_var("HS_PROJECT_ROOT", &root) };
    let esc = termexec::run_readonly(
        &root,
        "echo evil >> task.txt && echo WROTE || echo REFUSED; echo ok > /tmp/v-scratch.txt && echo SCRATCH",
        10,
    );
    unsafe { std::env::remove_var("HS_PROJECT_ROOT") };
    let out = esc["stdout"].as_str().unwrap_or("");
    assert!(
        out.contains("REFUSED"),
        "verifier parity: task files read-only by mechanism: {esc}"
    );
    assert!(out.contains("SCRATCH"), "verifier scratch persists: {esc}");
    assert_eq!(
        std::fs::read_to_string(root.join("task.txt")).unwrap(),
        "graded artifact",
        "the verifier could not modify the submission"
    );
}

#[test]
fn b4_platform_probe_passes_on_this_dev_box() {
    termexec::sandbox_probe().expect("the dev box confines missions (bwrap)");
}
