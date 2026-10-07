//! RED (2026-10-06, real-model t1 rerun): the model finished the task in
//! ~3 min, then the declared-check re-run failed for 9 more minutes. The
//! model's shell (termexec, bwrap) runs with a scrubbed env: HOME=<project
//! root>, PATH=<root>/.cargo/bin:<root>/.local/bin:... . The checker ran
//! the same declared command with the HARNESS's own env, so a tool the
//! model installed under <root>/.local (user-site pytest) was invisible to
//! it, the check failed for an environment reason, and the model looped.
//! THE LAW: a declared check runs in the SAME environment the model's
//! shell used. Own binary: HS_PROJECT_ROOT / HS_SELFCHECK_DIRECT are
//! process-global.

use std::os::unix::fs::PermissionsExt;

static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

#[test]
fn declared_check_sees_the_models_shell_environment() {
    let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let root = std::env::temp_dir().join(format!("hs-selfcheck-env-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let task = root.join("task");
    std::fs::create_dir_all(task.join(".hs")).unwrap();
    // A tool the model "installed" into its user-site bin.
    let bin = root.join(".local/bin");
    std::fs::create_dir_all(&bin).unwrap();
    let tool = bin.join("model-installed-tool");
    std::fs::write(&tool, "#!/bin/sh\necho tool-ran\n").unwrap();
    std::fs::set_permissions(&tool, std::fs::Permissions::from_mode(0o755)).unwrap();
    std::fs::write(
        task.join(".hs/checks"),
        "model-installed-tool\ntest \"$HOME\" = \"$HS_EXPECT_HOME\"\n",
    )
    .unwrap();
    let canon = std::fs::canonicalize(&root).unwrap();
    unsafe {
        std::env::set_var("HS_PROJECT_ROOT", &root);
        std::env::set_var("HS_SELFCHECK_DIRECT", "1");
        // bwrap --clearenv drops this; compare against the root literal instead.
    }
    std::fs::write(
        task.join(".hs/checks"),
        format!("model-installed-tool\ntest \"$HOME\" = \"{}\"\n", canon.display()),
    )
    .unwrap();
    let v = hs_loop::selfcheck::check(&task);
    assert_eq!(v["passed"], true, "checker must run in the model's env: {v}");
}

#[test]
fn failing_check_still_fails_in_the_shared_env() {
    let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    // The fix must not weaken the checker: a genuinely red check is red.
    let root = std::env::temp_dir().join(format!("hs-selfcheck-red-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let task = root.join("task");
    std::fs::create_dir_all(task.join(".hs")).unwrap();
    std::fs::write(task.join(".hs/checks"), "echo boom; exit 3\n").unwrap();
    unsafe {
        std::env::set_var("HS_PROJECT_ROOT", &root);
        std::env::set_var("HS_SELFCHECK_DIRECT", "1");
    }
    let v = hs_loop::selfcheck::check(&task);
    assert_eq!(v["passed"], false);
    let e = v["error"].as_str().unwrap();
    assert!(e.contains("exited Some(3)") && e.contains("boom"), "{e}");
}
