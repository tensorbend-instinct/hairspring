//! RED (stranger-path live burn 2026-09-09): when a plugin dies on its
//! first call, the operator's console showed only `plugin exited (EOF)` -
//! the plugin's real dying words (a missing API key message naming the env
//! var) sat in a stderr log nothing pointed at. A dying plugin's stderr
//! tail must ride the error so the failure says what to fix.

use hs_kernel::*;

const FIXTURE: &str = env!("CARGO_BIN_EXE_hs-fixture-plugin");

#[test]
fn dead_plugins_stderr_tail_reaches_the_error() {
    let dir = tempfile::tempdir().unwrap();
    let log = tempfile::tempdir().unwrap();
    let path = dir.path().join("hairspring.toml");
    std::fs::write(
        &path,
        format!(
            r#"
[[tools]]
name = "dying"
command = ["{FIXTURE}", "dies-stderr"]
subjects = ["*"]
"#
        ),
    )
    .unwrap();
    let k = Kernel::load_with_log(&path, log.path()).unwrap();
    let err = k
        .call_tool("anyone", "dying", serde_json::json!({}))
        .unwrap_err();
    let msg = err.to_string();
    assert!(
        msg.contains("dying-words-marker"),
        "the error carries the plugin's dying words (live: naked `plugin \
         exited (EOF)`): {msg}"
    );
}
