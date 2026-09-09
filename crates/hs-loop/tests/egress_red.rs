//! RED (2026-09-06): `HS_SWE_NET=off` - the egress-off re-baseline switch.
//! Eric's contamination caveat (BASELINE-2026-09-06): >=3 passes at 98-100%
//! gold-patch line overlap via network fetch; ~29 runs touched GitHub. The
//! re-baseline needs network OFF inside the mission (repo.exec sandbox AND
//! the host-side f2p evaluator), with the prompt saying so.
use std::path::Path;
use std::sync::Mutex;

/// `HS_SWE_NET` is process-global; tests in this binary run in threads, so
/// every env-touching test serializes on this lock (flaky without it).
static ENV_LOCK: Mutex<()> = Mutex::new(());

#[test]
fn sandbox_omits_share_net_when_egress_off() {
    let _g = ENV_LOCK.lock().unwrap();
    // FIXME: Audit that the environment access only happens in single-threaded code.
    unsafe { std::env::set_var("HS_SWE_NET", "off") };
    let argv = hs_loop::repexec::sandbox_argv(
        Path::new("/tmp/x"),
        Path::new("/tmp/o"),
        Path::new("/tmp/e"),
        "true",
    );
    // FIXME: Audit that the environment access only happens in single-threaded code.
    unsafe { std::env::remove_var("HS_SWE_NET") };
    assert!(
        !argv.iter().any(|a| a == "--share-net"),
        "egress off: sandbox must NOT share the host net: {argv:?}"
    );
}

#[test]
fn sandbox_shares_net_by_default() {
    let _g = ENV_LOCK.lock().unwrap();
    // FIXME: Audit that the environment access only happens in single-threaded code.
    unsafe { std::env::remove_var("HS_SWE_NET") };
    let argv = hs_loop::repexec::sandbox_argv(
        Path::new("/tmp/x"),
        Path::new("/tmp/o"),
        Path::new("/tmp/e"),
        "true",
    );
    assert!(
        argv.iter().any(|a| a == "--share-net"),
        "default stays Network: ON until the re-baseline is ordered"
    );
}

#[test]
fn host_eval_wraps_unshare_net_when_egress_off() {
    let _g = ENV_LOCK.lock().unwrap();
    // FIXME: Audit that the environment access only happens in single-threaded code.
    unsafe { std::env::set_var("HS_SWE_NET", "off") };
    let w = hs_loop::repexec::host_command_wrapper("pytest t -x");
    // FIXME: Audit that the environment access only happens in single-threaded code.
    unsafe { std::env::remove_var("HS_SWE_NET") };
    assert!(
        w.contains("unshare -n"),
        "host-side f2p must lose egress too: {w}"
    );
}

#[test]
fn host_eval_unwrapped_by_default() {
    let _g = ENV_LOCK.lock().unwrap();
    // FIXME: Audit that the environment access only happens in single-threaded code.
    unsafe { std::env::remove_var("HS_SWE_NET") };
    let w = hs_loop::repexec::host_command_wrapper("pytest t -x");
    assert!(
        !w.contains("unshare -n"),
        "default host eval unchanged: {w}"
    );
}

#[test]
fn prompt_states_network_off_when_egress_off() {
    let _g = ENV_LOCK.lock().unwrap();
    let args = hs_loop::sweprompt::PromptArgs {
        ws: "/tmp/ws".into(),
        problem_statement: "p".into(),
        fail_to_pass: vec!["t".into()],
        repo_layout: "src/main.rs\n".into(),
        nudge: String::new(),
        answer_path: "/tmp/a".into(),
        orientation: String::new(),
        mcp_tools: String::new(),
    };
    // FIXME: Audit that the environment access only happens in single-threaded code.
    unsafe { std::env::set_var("HS_SWE_NET", "off") };
    let p = hs_loop::sweprompt::build_mission_prompt(None, &args);
    // FIXME: Audit that the environment access only happens in single-threaded code.
    unsafe { std::env::remove_var("HS_SWE_NET") };
    assert!(
        p.contains("Network: OFF"),
        "egress-off prompt must say so: {}",
        &p[..p.len().min(500)]
    );
    assert!(!p.contains("Network: ON"), "no contradictory line");
    let p_on = hs_loop::sweprompt::build_mission_prompt(None, &args);
    assert!(p_on.contains("Network: ON"), "default unchanged");
}
