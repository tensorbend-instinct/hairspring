//! Phase 1 RED (design D4): plugin supervisor contract.
//! A dead plugin must be respawned from None with bounded retries; after 3
//! strikes the slot reports PluginDead naming the plugin and the real cause;
//! a slot that failed out is NOT poisoned - it recovers when the plugin can
//! spawn again. A hung plugin is killed by a per-call lease (the heartbeat).

use hs_kernel::*;

const FIXTURE: &str = env!("CARGO_BIN_EXE_hs-fixture-plugin");

fn write_config(dir: &std::path::Path, body: &str) -> std::path::PathBuf {
    let p = dir.join("hairspring.toml");
    std::fs::write(&p, body).unwrap();
    p
}

/// Control: a plugin that crashes once is respawned and the call succeeds
/// (the pre-existing single-restart behavior must keep working).
#[test]
fn crashed_plugin_recovers_and_serves() {
    let _ = std::fs::remove_file(std::env::temp_dir().join("hs-fixture-flaky-once"));
    let dir = tempfile::tempdir().unwrap();
    let path = write_config(
        dir.path(),
        &format!(
            r#"
[[tools]]
name = "flaky"
command = ["{FIXTURE}", "flaky-tool"]
subjects = ["*"]
"#
        ),
    );
    let k = Kernel::load(&path).unwrap();
    let out = k
        .call_tool("anyone", "flaky", serde_json::json!({}))
        .unwrap();
    assert_eq!(out.output["output"], "flaky-ok");
}

/// T3a: a plugin that dies on every call produces, after bounded attempts,
/// a PluginDead error that names the plugin and the real cause - not a
/// nameless stale error, and not an infinite respawn loop.
#[test]
fn dead_plugin_errors_with_name_and_bounded_attempts() {
    let dir = tempfile::tempdir().unwrap();
    let spawns = dir.path().join("spawns.log");
    let path = write_config(
        dir.path(),
        &format!(
            r#"
[[tools]]
name = "zombie"
command = ["{FIXTURE}", "dies-always", "{}", "{}"]
subjects = ["*"]
"#,
            spawns.display(),
            spawns.display()
        ),
    );
    let k = Kernel::load(&path).unwrap();
    let err = k
        .call_tool("anyone", "zombie", serde_json::json!({}))
        .expect_err("a permanently dead plugin must fail the call");
    let msg = format!("{err:?}");
    assert!(msg.contains("PluginDead"), "want PluginDead, got: {msg}");
    assert!(msg.contains("zombie"), "error must name the plugin: {msg}");
    // spawn accounting: load (describe) + one call attempt on that proc +
    // two respawns = 3 total spawns, then strikes out. Never a hot loop.
    let n = std::fs::read_to_string(&spawns)
        .unwrap_or_default()
        .lines()
        .count();
    assert_eq!(n, 3, "load + 2 respawns, then PluginDead; got {n} spawns");
}

/// T3b (the 17117 regression): after a slot fails out, it is not poisoned.
/// When the plugin can spawn again, the very next call succeeds - the
/// supervisor respawns from None instead of erroring "not spawned" forever.
#[test]
fn failed_out_slot_recovers_when_plugin_returns() {
    let dir = tempfile::tempdir().unwrap();
    let flag = dir.path().join("alive.flag");
    let path = write_config(
        dir.path(),
        &format!(
            r#"
[[tools]]
name = "revivable"
command = ["{FIXTURE}", "dies-unless-flag", "{}", "{}"]
subjects = ["*"]
"#,
            flag.display(),
            flag.display()
        ),
    );
    let k = Kernel::load(&path).unwrap();
    let first = k.call_tool("anyone", "revivable", serde_json::json!({}));
    assert!(first.is_err(), "dead while the flag is absent");
    std::fs::write(&flag, b"alive").unwrap();
    let second = k.call_tool("anyone", "revivable", serde_json::json!({}));
    assert!(
        second.is_ok(),
        "slot must respawn from None once the plugin is back: {second:?}"
    );
    assert_eq!(second.unwrap().output["output"], "revived");
}

/// T3c: a hung plugin is killed by the per-call lease and the strike is
/// counted; the supervisor does not block the kernel forever on a read.
#[test]
fn hung_plugin_is_killed_by_lease() {
    let dir = tempfile::tempdir().unwrap();
    let path = write_config(
        dir.path(),
        &format!(
            r#"
[[tools]]
name = "sleeper"
command = ["{FIXTURE}", "hang-tool"]
subjects = ["*"]
lease_secs = 1
"#
        ),
    );
    let k = Kernel::load(&path).unwrap();
    let t0 = std::time::Instant::now();
    let err = k
        .call_tool("anyone", "sleeper", serde_json::json!({}))
        .expect_err("a hung plugin must fail via lease, not block forever");
    let elapsed = t0.elapsed();
    assert!(
        elapsed < std::time::Duration::from_secs(20),
        "3 strikes at 1s lease must finish in seconds, took {elapsed:?}"
    );
    let msg = format!("{err:?}");
    assert!(
        msg.contains("PluginDead"),
        "want PluginDead after strikes: {msg}"
    );
}

/// Exploration is never punished (Eric 2026-09-05, ab2/17123): a
/// well-formed {"error": ...} response from a HEALTHY plugin process is an
/// application-level answer, not a crash. It must return as PluginApp, cost
/// zero strikes, and never kill the process - no matter how many times in a
/// row it happens. Environment probes (pwd, which, echo, installs) hit
/// exactly this path when a tool rejects the args.
#[test]
fn app_errors_never_strike_and_never_kill_the_process() {
    let dir = tempfile::tempdir().unwrap();
    let state = dir.path().join("usageerr.log");
    let path = write_config(
        dir.path(),
        &format!(
            r#"
[[tools]]
name = "usageerr"
command = ["{FIXTURE}", "usage-error", "{}"]
subjects = ["*"]
"#,
            state.display()
        ),
    );
    let k = Kernel::load(&path).unwrap();
    // 5 in a row: three would strike out a crash-class failure
    for i in 1..=5 {
        let err = k
            .call_tool("anyone", "usageerr", serde_json::json!({}))
            .expect_err("an app-level error must surface as an error value");
        let msg = format!("{err:?}");
        assert!(
            msg.contains("PluginApp"),
            "call {i}: want PluginApp (no strike), got: {msg}"
        );
        assert!(
            !msg.contains("PluginDead"),
            "call {i}: app errors must never strike out: {msg}"
        );
    }
    // the process was never killed: exactly one spawn, five calls served
    let lines: Vec<String> = std::fs::read_to_string(&state)
        .unwrap()
        .lines()
        .map(|l| l.to_string())
        .collect();
    let spawns = lines.iter().filter(|l| *l == "spawn").count();
    let calls = lines.iter().filter(|l| *l == "call").count();
    assert_eq!(
        spawns, 1,
        "app errors must never kill the process; spawns: {lines:?}"
    );
    assert_eq!(calls, 5, "every call reached the live process: {lines:?}");
}
