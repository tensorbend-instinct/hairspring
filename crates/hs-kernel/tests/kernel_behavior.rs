//! Gate-2 TDD: plugin kernel + Rails contract, written before the implementation.

use hs_kernel::*;

const FIXTURE: &str = env!("CARGO_BIN_EXE_hs-fixture-plugin");

/// Kernel spawns inherit this test process's env, and several fixtures read
/// process-global env (RAIL_LOG_FILE) or shared temp files. Tests in this
/// file therefore run serialized: parallel env mutation across tests raced
/// (observed 2026-09-05: interleaved rail-log lines, order assert flake).
static TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn write_config(dir: &std::path::Path, body: &str) -> std::path::PathBuf {
    let p = dir.join("hairspring.toml");
    std::fs::write(&p, body).unwrap();
    p
}

fn base_config() -> String {
    format!(
        r#"
[[tools]]
name = "echo"
command = ["{FIXTURE}", "echo-tool"]
subjects = ["*"]

[[tools]]
name = "alpha-only"
command = ["{FIXTURE}", "echo-tool", "alpha-only"]
subjects = ["alpha"]

[[models]]
name = "fake-v1"
command = ["{FIXTURE}", "fake-model"]
default = true

[[rails]]
name = "rail-c"
command = ["{FIXTURE}", "rail-c"]
hooks = ["call.post_tool"]
priority = 10

[[rails]]
name = "rail-a"
command = ["{FIXTURE}", "rail-a"]
hooks = ["call.post_tool"]
priority = 5

[[rails]]
name = "rail-b"
command = ["{FIXTURE}", "rail-b"]
hooks = ["call.post_tool"]
priority = 5
"#
    )
}

#[test]
fn config_loads_and_describes_plugins() {
    let _guard = TEST_LOCK.lock().unwrap();
    let dir = tempfile::tempdir().unwrap();
    let path = write_config(dir.path(), &base_config());
    let k = Kernel::load(&path).unwrap();
    let mut tools: Vec<_> = k
        .list_tools("alpha")
        .iter()
        .map(|t| t.name.clone())
        .collect();
    tools.sort();
    assert_eq!(tools, vec!["alpha-only", "echo"]);
    assert_eq!(k.list_tools("anyone").len(), 1);
    assert_eq!(k.list_models()[0].name, "fake-v1");
}

#[test]
fn describe_mismatch_is_rejected() {
    let _guard = TEST_LOCK.lock().unwrap();
    let dir = tempfile::tempdir().unwrap();
    let path = write_config(
        dir.path(),
        &format!(
            r#"
[[tools]]
name = "echo"
command = ["{FIXTURE}", "bogus"]
subjects = ["*"]
"#
        ),
    );
    let err = Kernel::load(&path).unwrap_err();
    assert!(matches!(err, KernelError::Protocol(_)), "{err:?}");
}

#[test]
fn visibility_gating_filters_by_subject() {
    let _guard = TEST_LOCK.lock().unwrap();
    let dir = tempfile::tempdir().unwrap();
    let path = write_config(dir.path(), &base_config());
    let k = Kernel::load(&path).unwrap();
    // alpha sees both tools; beta sees only echo
    assert_eq!(k.list_tools("beta").len(), 1);
    assert_eq!(k.list_tools("alpha").len(), 2);
    let err = k
        .call_tool("beta", "alpha-only", serde_json::json!({"text": "hi"}))
        .unwrap_err();
    assert!(matches!(err, KernelError::Gated { .. }), "{err:?}");
    k.call_tool("alpha", "alpha-only", serde_json::json!({"text": "hi"}))
        .unwrap();
}

#[test]
fn tool_call_executes_and_returns_output() {
    let _guard = TEST_LOCK.lock().unwrap();
    let dir = tempfile::tempdir().unwrap();
    let path = write_config(dir.path(), &base_config());
    let k = Kernel::load(&path).unwrap();
    let out = k
        .call_tool("anyone", "echo", serde_json::json!({"text": "hairspring"}))
        .unwrap();
    assert_eq!(out.output["output"], "hairspring");
    assert!(out.latency_ms < 5000);
}

#[test]
fn model_call_returns_completion_counts_and_cost() {
    let _guard = TEST_LOCK.lock().unwrap();
    let dir = tempfile::tempdir().unwrap();
    let path = write_config(dir.path(), &base_config());
    let k = Kernel::load(&path).unwrap();
    let out = k.call_model("anyone", None, "hello").unwrap();
    assert_eq!(out.completion, "fake-completion:olleh");
    assert!(out.input_tokens >= 1);
    assert_eq!(out.output_tokens, 7);
    assert_eq!(out.cost_usd_micros, 1300);
}

#[test]
fn rails_fire_in_priority_order_with_name_tiebreak() {
    let _guard = TEST_LOCK.lock().unwrap();
    let dir = tempfile::tempdir().unwrap();
    let rail_log = dir.path().join("rail.log");
    std::env::set_var("RAIL_LOG_FILE", &rail_log);
    let path = write_config(dir.path(), &base_config());
    let k = Kernel::load(&path).unwrap();
    k.call_tool("anyone", "echo", serde_json::json!({"text": "x"}))
        .unwrap();
    let order = std::fs::read_to_string(&rail_log).unwrap();
    let lines: Vec<&str> = order.lines().collect();
    // priority 5 (rail-a, rail-b: name tiebreak) before priority 10 (rail-c)
    assert_eq!(
        lines,
        vec![
            "rail-a:call.post_tool",
            "rail-b:call.post_tool",
            "rail-c:call.post_tool",
        ],
        "hook order"
    );
}

#[test]
fn rail_failure_is_contained_and_logged() {
    let _guard = TEST_LOCK.lock().unwrap();
    let dir = tempfile::tempdir().unwrap();
    let cfg = base_config()
        + &format!(
            r#"
[[rails]]
name = "rail-crash"
command = ["{FIXTURE}", "rail-crash"]
hooks = ["call.post_tool"]
priority = 99
"#
        );
    let path = write_config(dir.path(), &cfg);
    let mut logdir = tempfile::tempdir().unwrap();
    let k = Kernel::load_with_log(&path, logdir.path()).unwrap();
    // the tool call still succeeds even though a rail died
    let out = k
        .call_tool("anyone", "echo", serde_json::json!({"text": "x"}))
        .unwrap();
    assert_eq!(out.output["output"], "x");
    // and the failure is on the canonical record
    let events = read_only_stream(logdir.path());
    assert!(
        events
            .iter()
            .any(|e| e.kind == hs_core::EventKind::Observation),
        "rail failure not logged"
    );
    let _ = &mut logdir;
}

#[test]
fn crashed_plugin_process_is_restarted() {
    let _guard = TEST_LOCK.lock().unwrap();
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
    assert_eq!(
        out.output["output"], "flaky-ok",
        "kernel did not restart the crashed plugin"
    );
}

#[test]
fn hot_reload_adds_capability_without_restart() {
    let _guard = TEST_LOCK.lock().unwrap();
    let dir = tempfile::tempdir().unwrap();
    let path = write_config(
        dir.path(),
        &format!(
            r#"
[[tools]]
name = "echo"
command = ["{FIXTURE}", "echo-tool"]
subjects = ["*"]
"#
        ),
    );
    let mut k = Kernel::load(&path).unwrap();
    assert!(k
        .call_tool("anyone", "upper", serde_json::json!({}))
        .is_err());
    std::thread::sleep(std::time::Duration::from_millis(20)); // distinct mtime tick
                                                              // config changes on disk; harness code and process unchanged
    std::fs::write(
        &path,
        format!(
            r#"
[[tools]]
name = "echo"
command = ["{FIXTURE}", "echo-tool"]
subjects = ["*"]

[[tools]]
name = "upper"
command = ["{FIXTURE}", "echo-tool", "upper"]
subjects = ["*"]

[[models]]
name = "fake-v2"
command = ["{FIXTURE}", "fake-model", "fake-v2"]
default = true
"#
        ),
    )
    .unwrap();
    // mtime granularity: ensure the change is visible
    std::thread::sleep(std::time::Duration::from_millis(20));
    assert!(k.reload_if_changed().unwrap(), "reload not detected");
    let out = k
        .call_tool("anyone", "upper", serde_json::json!({"text": "new"}))
        .unwrap();
    assert_eq!(out.output["output"], "new");
    let m = k.call_model("anyone", Some("fake-v2"), "hi").unwrap();
    assert!(m.completion.starts_with("fake-completion:"));
}

#[test]
fn calls_are_recorded_in_the_event_log() {
    let _guard = TEST_LOCK.lock().unwrap();
    let dir = tempfile::tempdir().unwrap();
    let logdir = tempfile::tempdir().unwrap();
    let path = write_config(dir.path(), &base_config());
    let k = Kernel::load_with_log(&path, logdir.path()).unwrap();
    k.call_tool("anyone", "echo", serde_json::json!({"text": "logged"}))
        .unwrap();
    k.call_model("anyone", None, "count me").unwrap();
    let events = read_only_stream(logdir.path());
    assert!(events
        .iter()
        .any(|e| e.kind == hs_core::EventKind::ToolCall));
    let mc = events
        .iter()
        .find(|e| e.kind == hs_core::EventKind::ModelCall)
        .expect("model_call event");
    assert_eq!(
        mc.cost_usd_micros, 1300,
        "cost from the plugin must land on the canonical record"
    );
    let sid = events[0].stream_id;
    hs_log::verify_stream(logdir.path(), sid).unwrap();
}
