//! RED: repo.exec scratch-shell mode (Eric 2026-09-05, post-verify17092 kill).
//! The verify17092 run burned 4 model calls on "no answer path" - the model
//! uses repo.exec as a general shell (git log, grep, pwd) with NO candidate
//! diff, and the contract rejected anything without a diff/path. New contract:
//! no diff + no path + no `HS_SWE_ANSWER` = scratch shell against a pristine
//! clone of the workspace (`exit_code/stdout/stderr`, applied=false,
//! scratch=true). The diff path (args.diff / args.path / `HS_SWE_ANSWER`) stays
//! the candidate-testing path, unchanged.
//!
//! Levels:
//! - unit: `hs_loop::repexec::run_sandboxed_no_patch` against a real git ws
//! - mission: a scripted model uses repo.exec as a general shell through the
//!   real loop/kernel/plugin path; the stream must show `exit_code` 0 and no
//!   "$error" (this fails RED against the old plugin contract)

use hs_loop::*;

const ANSWER: &str = env!("CARGO_BIN_EXE_hs-plugin-answer");
const CHECKER: &str = env!("CARGO_BIN_EXE_hs-plugin-checker");
const SEQMODEL: &str = env!("CARGO_BIN_EXE_hs-plugin-scripted");
const REPOEXEC: &str = env!("CARGO_BIN_EXE_hs-plugin-repoexec");

fn mk_ws() -> tempfile::TempDir {
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
    git(&["init", "-q"]);
    git(&["add", "."]);
    git(&["commit", "-qm", "init"]);
    dir
}

#[test]
fn scratch_shell_runs_general_commands_against_pristine_clone() {
    // FIXME: Audit that the environment access only happens in single-threaded code.
    unsafe { std::env::remove_var("HS_SWE_ANSWER") };
    let ws = mk_ws();
    let r = hs_loop::repexec::run_sandboxed_no_patch(
        ws.path(),
        "git log --oneline | head -1; pwd; grep -c broken code.txt",
        30,
    );
    assert_eq!(r["exit_code"], 0, "scratch shell must succeed: {r}");
    assert_eq!(r["applied"], false, "no patch applied in scratch mode: {r}");
    assert_eq!(r["scratch"], true, "result must label scratch mode: {r}");
    let out = r["stdout"].as_str().unwrap();
    assert!(
        out.contains("init"),
        "git log works inside the sandbox (self-contained clone): {out}"
    );
    assert!(out.contains('1'), "grep sees HEAD content: {out}");
    assert!(r.get("$error").is_none(), "no contract error: {r}");
    // live workspace untouched
    assert_eq!(
        std::fs::read_to_string(ws.path().join("code.txt")).unwrap(),
        "broken\n"
    );
}

#[test]
fn scratch_shell_enforces_timeout() {
    // FIXME: Audit that the environment access only happens in single-threaded code.
    unsafe { std::env::remove_var("HS_SWE_ANSWER") };
    let ws = mk_ws();
    let t0 = std::time::Instant::now();
    let r = hs_loop::repexec::run_sandboxed_no_patch(ws.path(), "sleep 30", 2);
    assert!(t0.elapsed().as_secs() < 15, "timeout must fire");
    assert_eq!(r["timed_out"], true, "{r}");
    assert_eq!(r["scratch"], true, "{r}");
}

/// Mission level: the verify17092 failure shape, replayed. Scripted model
/// calls repo.exec with a bare shell command (no diff, no path, no
/// `HS_SWE_ANSWER`) exactly as kimi-k3 did; under the old contract the result
/// is {"$error": "no answer path..."}. Then it verifies a candidate diff and
/// writes the answer, so the mission passes. Asserts on the event stream.
#[test]
fn mission_model_may_use_repoexec_as_general_shell() {
    // FIXME: Audit that the environment access only happens in single-threaded code.
    unsafe { std::env::remove_var("HS_SWE_ANSWER") };
    let dir = tempfile::tempdir().unwrap();
    let log = tempfile::tempdir().unwrap();
    let ws = mk_ws();
    // FIXME: Audit that the environment access only happens in single-threaded code.
    unsafe { std::env::set_var("HS_SWE_WORKSPACE", ws.path()) };
    let answer = log.path().join("work").join("task-0").join("answer.txt");
    let diff = "```diff\n--- a/code.txt\n+++ b/code.txt\n@@ -1 +1 @@\n-broken\n+fixed\n```";
    let script = dir.path().join("script.jsonl");
    std::fs::write(
        &script,
        format!(
            "{{\"tool\":\"repo.exec\",\"args\":{{\"command\":\"git log --oneline | head -1\"}}}}\n{{\"tool\":\"repo.exec\",\"args\":{{\"command\":\"cat code.txt\",\"diff\":\"{}\"}}}}\n{{\"tool\":\"answer.write\",\"args\":{{\"path\":\"{}\",\"content\":\"TOKEN-0-SECRET\"}}}}",
            diff.replace('\n', "\\n"),
            answer.display()
        ),
    )
    .unwrap();
    // FIXME: Audit that the environment access only happens in single-threaded code.
    unsafe { std::env::set_var("HS_SEQMODEL_SCRIPT", &script) };
    let config = dir.path().join("hairspring.toml");
    std::fs::write(
        &config,
        format!(
            r#"
[[tools]]
name = "answer.write"
command = ["{ANSWER}"]
subjects = ["*"]

[[tools]]
name = "checker.run"
command = ["{CHECKER}"]
subjects = ["*"]

[[tools]]
name = "repo.exec"
command = ["{REPOEXEC}"]
subjects = ["*"]

[[models]]
name = "scripted"
command = ["{SEQMODEL}"]
default = true
"#
        ),
    )
    .unwrap();
    let kernel = hs_kernel::Kernel::load(&config).unwrap();
    let mut l = InnerLoop::new(kernel, log.path(), true, 8).unwrap();
    let r = l.run_mission("task-0").unwrap();
    assert!(r.passed, "verify-then-write flow passes: {r:?}");

    let streams = log.path().join("streams");
    let sid = std::fs::read_dir(&streams)
        .unwrap()
        .next()
        .unwrap()
        .unwrap();
    let sid = uuid::Uuid::parse_str(sid.file_name().to_str().unwrap()).unwrap();
    let reader = hs_log::StreamReader::open(log.path(), sid).unwrap();
    let events = reader.events().unwrap();
    let mut exec_results = events.iter().filter_map(|e| {
        let p = String::from_utf8_lossy(&reader.resolve_payload(e).unwrap()).to_string();
        p.contains("\"plugin\":\"repo.exec\"").then_some(p)
    });
    let scratch_call = exec_results
        .next()
        .expect("first repo.exec ToolCall on the stream");
    assert!(
        !scratch_call.contains("$error"),
        "bare shell command must NOT hit the no-answer-path contract error: {scratch_call}"
    );
    assert!(
        scratch_call.contains("\"exit_code\":0"),
        "scratch shell ran the command: {scratch_call}"
    );
    assert!(
        scratch_call.contains("\"scratch\":true"),
        "result labels scratch mode: {scratch_call}"
    );
    assert!(
        scratch_call.contains("init"),
        "git log output reached the model: {scratch_call}"
    );

    // FIXME: Audit that the environment access only happens in single-threaded code.
    unsafe { std::env::remove_var("HS_SWE_WORKSPACE") };
    // FIXME: Audit that the environment access only happens in single-threaded code.
    unsafe { std::env::remove_var("HS_SEQMODEL_SCRIPT") };
}
