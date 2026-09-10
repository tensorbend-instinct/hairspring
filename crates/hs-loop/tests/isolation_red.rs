//! Hostile escape tests for mission confinement (RED first).
//!
//! A mission's shell access (term.exec) and answer writes (answer.submit)
//! must be confined BY MECHANISM to the configured project root
//! (HS_PROJECT_ROOT, exported by the harness; `--project-dir` on hs-repl):
//! - absolute reads outside the project root cannot resolve
//! - `..` escapes cannot resolve
//! - harness env (HS_* config, keys, model names) is not visible inside
//!   mission shell commands
//! - writes INSIDE the project root still persist (the machine-state
//!   contract missions are graded on)
//! - a workdir outside the project root is refused
//! - when the sandbox binary is absent the call fails CLOSED
//!
//! The drive is the real plugin binary over the real wire protocol, the
//! same path a mission's ToolCall takes (wire_tool_env -> plugin spawn ->
//! tool.call), not a mocked shell.

use serde_json::Value;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

fn tmpdir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("hs-iso-red-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// One JSON-RPC tool.call against a plugin binary; returns the single
/// response frame.
fn plugin_call(exe: &str, envs: &[(&str, String)], args: Value) -> Value {
    let mut cmd = Command::new(exe);
    for (k, v) in envs {
        cmd.env(k, v);
    }
    let mut child = cmd
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn plugin");
    let req = serde_json::json!({"id": 1, "method": "tool.call", "params": {"args": args}});
    let mut stdin = child.stdin.take().unwrap();
    stdin.write_all(req.to_string().as_bytes()).unwrap();
    stdin.write_all(b"\n").unwrap();
    drop(stdin);
    let out = child.wait_with_output().unwrap();
    let text = String::from_utf8_lossy(&out.stdout);
    let line = text.lines().last().expect("plugin must answer");
    serde_json::from_str(line).expect("response frame must be json")
}

fn termexec(root: &Path, cmd: &str, extra: &[(&str, String)]) -> Value {
    let work = root.join("work");
    std::fs::create_dir_all(&work).unwrap();
    let mut envs: Vec<(&str, String)> = vec![
        ("HS_PROJECT_ROOT", root.to_string_lossy().into()),
        ("HS_TERM_WORKDIR", work.to_string_lossy().into()),
        ("HS_TERM_EXEC_TIMEOUT_SECS", "20".into()),
    ];
    envs.extend_from_slice(extra);
    plugin_call(
        env!("CARGO_BIN_EXE_hs-plugin-termexec"),
        &envs,
        serde_json::json!({"command": cmd}),
    )
}

fn result_of(resp: &Value) -> String {
    serde_json::to_string(resp).unwrap()
}

#[test]
fn absolute_read_outside_project_is_blocked() {
    let tmp = tmpdir("absread");
    let secret = tmp.join("secret.txt");
    std::fs::write(&secret, "TOPSECRET-7788").unwrap();
    let root = tmp.join("proj");
    let resp = termexec(&root, &format!("cat {}", secret.display()), &[]);
    let body = result_of(&resp);
    assert!(
        !body.contains("TOPSECRET-7788"),
        "mission shell read a secret OUTSIDE the project root: {body}"
    );
}

#[test]
fn dotdot_escape_outside_project_is_blocked() {
    let tmp = tmpdir("dotdot");
    std::fs::write(tmp.join("secret.txt"), "TOPSECRET-9988").unwrap();
    let root = tmp.join("proj");
    let resp = termexec(&root, "cat ../../secret.txt", &[]);
    let body = result_of(&resp);
    assert!(
        !body.contains("TOPSECRET-9988"),
        "mission shell escaped the project root with ..: {body}"
    );
}

#[test]
fn harness_env_is_not_visible_inside_mission_shell() {
    let tmp = tmpdir("envscrub");
    let root = tmp.join("proj");
    let resp = termexec(
        &root,
        "env",
        &[
            ("HS_DEEPSEEK_MODEL", "marker-model-1337".into()),
            ("HS_HARNESS_MARKER", "marker-harness-1337".into()),
        ],
    );
    let body = result_of(&resp);
    assert!(
        !body.contains("marker-model-1337") && !body.contains("marker-harness-1337"),
        "harness config env leaked into the mission shell: {body}"
    );
}

#[test]
fn writes_inside_project_persist() {
    let tmp = tmpdir("persist");
    let root = tmp.join("proj");
    let target = root.join("work").join("state.txt");
    let resp = termexec(&root, &format!("echo persisted-55 > {}", target.display()), &[]);
    let body = result_of(&resp);
    assert!(
        !body.contains("$error") && !body.contains("\"error\""),
        "a write inside the project root must not be blocked: {body}"
    );
    assert_eq!(
        std::fs::read_to_string(&target).unwrap().trim(),
        "persisted-55",
        "machine-state contract: writes inside the project must persist across calls"
    );
}

#[test]
fn workdir_outside_project_is_refused() {
    let tmp = tmpdir("badworkdir");
    let root = tmp.join("proj");
    std::fs::create_dir_all(&root).unwrap();
    let resp = plugin_call(
        env!("CARGO_BIN_EXE_hs-plugin-termexec"),
        &[
            ("HS_PROJECT_ROOT", root.to_string_lossy().into()),
            ("HS_TERM_WORKDIR", "/etc".into()),
        ],
        serde_json::json!({"command": "id"}),
    );
    let body = result_of(&resp);
    assert!(
        body.contains("\"error\"") || body.contains("$error"),
        "a term.exec workdir outside the project root must be refused, got: {body}"
    );
}

#[test]
fn missing_sandbox_fails_closed() {
    let tmp = tmpdir("nobwrap");
    let root = tmp.join("proj");
    std::fs::create_dir_all(root.join("work")).unwrap();
    let empty = tmp.join("empty-bin");
    std::fs::create_dir_all(&empty).unwrap();
    let resp = plugin_call(
        env!("CARGO_BIN_EXE_hs-plugin-termexec"),
        &[
            ("HS_PROJECT_ROOT", root.to_string_lossy().into()),
            ("HS_TERM_WORKDIR", root.join("work").to_string_lossy().into()),
            ("PATH", empty.to_string_lossy().into()),
        ],
        serde_json::json!({"command": "id"}),
    );
    let body = result_of(&resp);
    assert!(
        body.contains("bwrap"),
        "without the sandbox binary term.exec must fail CLOSED naming the sandbox, got: {body}"
    );
}

#[test]
fn answer_submit_outside_project_is_refused() {
    let tmp = tmpdir("answerscape");
    let root = tmp.join("proj");
    let work = root.join("work");
    std::fs::create_dir_all(&work).unwrap();
    let evil = tmp.join("evil-answer.txt");
    let resp = plugin_call(
        env!("CARGO_BIN_EXE_hs-plugin-answersubmit"),
        &[
            ("HS_PROJECT_ROOT", root.to_string_lossy().into()),
            ("HS_SWE_WORKSPACE", work.to_string_lossy().into()),
            ("HS_ANSWER_RAW", "1".into()),
        ],
        serde_json::json!({"path": evil.to_string_lossy(), "summary": "hostile"}),
    );
    let body = result_of(&resp);
    assert!(
        body.contains("\"error\"") || body.contains("$error"),
        "answer.submit must refuse a path outside the project root, got: {body}"
    );
    assert!(
        !evil.exists(),
        "answer.submit wrote outside the project root: {}",
        evil.display()
    );
}

#[test]
fn project_root_resolution_prefers_flag_over_default() {
    // The lib-side resolution hs-repl's --project-dir drives: when
    // HS_PROJECT_ROOT is set (the flag's wiring) the mission work anchor
    // IS the project root; otherwise it stays <log_root>/work.
    let tmp = tmpdir("resolve");
    let log_root = tmp.join("run");
    let proj = tmp.join("my-project");
    std::fs::create_dir_all(&proj).unwrap();
    static ENV_MUTEX: std::sync::Mutex<()> = std::sync::Mutex::new(());
    let _g = ENV_MUTEX.lock().unwrap();
    unsafe { std::env::set_var("HS_PROJECT_ROOT", &proj) };
    let resolved = hs_loop::repl::resolve_work_dir(&log_root);
    unsafe { std::env::remove_var("HS_PROJECT_ROOT") };
    assert_eq!(resolved, proj.canonicalize().unwrap());
    let resolved_default = hs_loop::repl::resolve_work_dir(&log_root);
    assert_eq!(resolved_default, log_root.join("work"));
}
