//! RED (hostile review, 2026-09-09): three hot-reload/ledger defects.
//!
//! 1. `reload_if_changed` claims to "pick up config changes", but
//!    `apply_config` only spawns NEW names and retains the rest: editing
//!    an existing plugin's COMMAND leaves the old process serving under
//!    the same name - the config change is silently ignored.
//! 2. INVESTIGATED AND DISPROVEN (kept as a guard): a removed plugin's
//!    child process was suspected to leak (no Drop impl on `PluginProc`),
//!    but the stdin pipe closes on drop and the wire protocol's read loop
//!    EOFs, so well-behaved plugins reap themselves. The heartbeat test
//!    below pins that property so a future fixture/protocol change that
//!    breaks it is caught.
//! 3. `call_tool` records `ToolCall` events with canonical cost 0 while
//!    `call_model` records the model's real cost (and hs-loop/hs-goal
//!    record tool costs): a tool's reported `cost_usd_micros` never reaches
//!    the canonical field.
//!
//! Falsifiers: after a command edit + reload the NEW command's behavior
//! must show; after a removal reload the old process's heartbeat must
//! STOP; a `ToolCall` event's canonical cost must equal the tool's reported
//! `cost_usd_micros`.

use hs_kernel::*;
use std::sync::Mutex;

static TEST_LOCK: Mutex<()> = Mutex::new(());

const FIXTURE: &str = env!("CARGO_BIN_EXE_hs-fixture-plugin");

fn write_config(dir: &std::path::Path, body: &str) -> std::path::PathBuf {
    let path = dir.join("hairspring.toml");
    std::fs::write(&path, body).unwrap();
    path
}

#[test]
fn hot_reload_respawns_when_a_command_changes() {
    let _guard = TEST_LOCK.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let dir = tempfile::tempdir().unwrap();
    let path = write_config(
        dir.path(),
        &format!(
            r#"[[tools]]
name = "echo"
command = ["{FIXTURE}", "echo-tool"]
subjects = ["*"]
"#
        ),
    );
    let mut k = Kernel::load(&path).unwrap();
    let out = k
        .call_tool("anyone", "echo", serde_json::json!({"text": "ab"}))
        .unwrap();
    assert_eq!(out.output["output"], "ab");
    std::thread::sleep(std::time::Duration::from_millis(20)); // mtime tick
    std::fs::write(
        &path,
        format!(
            r#"[[tools]]
name = "echo"
command = ["{FIXTURE}", "shout-tool", "echo"]
subjects = ["*"]
"#
        ),
    )
    .unwrap();
    std::thread::sleep(std::time::Duration::from_millis(20));
    assert!(k.reload_if_changed().unwrap(), "reload not detected");
    let out = k
        .call_tool("anyone", "echo", serde_json::json!({"text": "ab"}))
        .unwrap();
    assert_eq!(
        out.output["output"], "AB",
        "command change was silently ignored: the old process still serves"
    );
}

#[test]
fn removed_plugin_process_reaps_itself_on_stdin_eof() {
    let _guard = TEST_LOCK.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let dir = tempfile::tempdir().unwrap();
    let beats = dir.path().join("beats.log");
    let path = write_config(
        dir.path(),
        &format!(
            r#"[[tools]]
name = "echo"
command = ["{FIXTURE}", "heartbeat-tool", "echo", "{}"]
subjects = ["*"]
"#,
            beats.display()
        ),
    );
    let mut k = Kernel::load(&path).unwrap();
    std::thread::sleep(std::time::Duration::from_millis(350));
    // Remove the tool from config and reload: the process must die.
    std::fs::write(&path, "").unwrap();
    std::thread::sleep(std::time::Duration::from_millis(20));
    assert!(k.reload_if_changed().unwrap(), "reload not detected");
    let count_at_removal = std::fs::read_to_string(&beats)
        .unwrap_or_default()
        .lines()
        .count();
    std::thread::sleep(std::time::Duration::from_millis(500));
    let count_after = std::fs::read_to_string(&beats)
        .unwrap_or_default()
        .lines()
        .count();
    assert_eq!(
        count_at_removal, count_after,
        "heartbeat continued after the plugin was removed: orphan process leak"
    );
}

#[test]
fn tool_cost_lands_in_the_canonical_event_field() {
    let _guard = TEST_LOCK.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let dir = tempfile::tempdir().unwrap();
    let logdir = tempfile::tempdir().unwrap();
    let path = write_config(
        dir.path(),
        &format!(
            r#"[[tools]]
name = "echo"
command = ["{FIXTURE}", "costed-tool"]
subjects = ["*"]
"#
        ),
    );
    let k = Kernel::load_with_log(&path, logdir.path()).unwrap();
    k.call_tool("anyone", "echo", serde_json::json!({"text": "x"}))
        .unwrap();
    let events = read_only_stream(logdir.path());
    let costs: Vec<i64> = events
        .iter()
        .filter(|e| e.kind == hs_core::EventKind::ToolCall)
        .map(|e| e.cost_usd_micros)
        .collect();
    assert_eq!(
        costs,
        vec![42],
        "tool-reported cost must land in the canonical field"
    );
}
