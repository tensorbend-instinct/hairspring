//! RED: terminate on a passing checker verdict (FIXLIST 2026-09-05 item 1).
//! A7 passed checker.run at seq 158 but kept burning for 22 steps / $1.46
//! because its goal evaluator went red ENVIRONMENTALLY (`goal::verify` re-runs
//! the f2p tests through the exec sandbox, which has no pytest) and D6 lets
//! a red goal evaluator veto the stop forever. New rule: a green checker.run
//! verdict ends the mission (the adversarial verifier veto still runs after
//! it); a red goal evaluator cannot hold a checker-passed mission to the
//! budget/wall guards.
//!
//! Level: mission. The scripted model writes the correct answer (checker
//! passes), then the script holds a repo.exec marker that must NEVER
//! execute. The goal evaluator is wired with `false` as its f2p command -
//! red by construction. Old code: goal red -> loop continues -> the marker
//! executes -> RED. New code: checker green stops the mission first.

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
fn checker_pass_terminates_even_when_goal_evaluator_is_red() {
    // FIXME: Audit that the environment access only happens in single-threaded code.
    unsafe { std::env::remove_var("HS_SWE_ANSWER") };
    let dir = tempfile::tempdir().unwrap();
    let log = tempfile::tempdir().unwrap();
    let ws = mk_ws();
    // FIXME: Audit that the environment access only happens in single-threaded code.
    unsafe { std::env::set_var("HS_SWE_WORKSPACE", ws.path()) };
    let answer = log.path().join("work").join("task-0").join("answer.txt");
    let script = dir.path().join("script.jsonl");
    let diff = "```diff\n--- a/code.txt\n+++ b/code.txt\n@@ -1 +1 @@\n-broken\n+fixed\n```";
    std::fs::write(
        &script,
        format!(
            "{{\"tool\":\"repo.exec\",\"args\":{{\"command\":\"cat code.txt\",\"diff\":\"{}\"}}}}\n{{\"tool\":\"answer.write\",\"args\":{{\"path\":\"{}\",\"content\":\"TOKEN-0-SECRET\"}}}}\n{{\"tool\":\"repo.exec\",\"args\":{{\"command\":\"echo MUST_NOT_EXECUTE\"}}}}",
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
    // goal evaluator that can NEVER go green: `false` exits 1 in the sandbox
    l.set_goal_evaluator(ws.path(), vec!["false".to_string()]);
    let r = l.run_mission("task-0").unwrap();
    assert!(
        r.passed,
        "a green checker verdict must end the mission even when the goal evaluator is red: {r:?}"
    );

    let streams = log.path().join("streams");
    let sid = std::fs::read_dir(&streams)
        .unwrap()
        .next()
        .unwrap()
        .unwrap();
    let sid = uuid::Uuid::parse_str(sid.file_name().to_str().unwrap()).unwrap();
    let reader = hs_log::StreamReader::open(log.path(), sid).unwrap();
    let events = reader.events().unwrap();
    let mut saw_checker_pass = false;
    for e in &events {
        let p = String::from_utf8_lossy(&reader.resolve_payload(e).unwrap()).to_string();
        assert!(!(p.contains("\"plugin\":\"repo.exec\"") && p.contains("MUST_NOT_EXECUTE")), "no tool call may execute after the checker pass: {p}");
        if p.contains("\"passed\":true") {
            saw_checker_pass = true;
        }
    }
    assert!(saw_checker_pass, "checker pass verdict on the stream");

    // FIXME: Audit that the environment access only happens in single-threaded code.
    unsafe { std::env::remove_var("HS_SWE_WORKSPACE") };
    // FIXME: Audit that the environment access only happens in single-threaded code.
    unsafe { std::env::remove_var("HS_SEQMODEL_SCRIPT") };
}
