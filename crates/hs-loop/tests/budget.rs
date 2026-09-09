//! Gate-8 budget enforcement on the inner loop (red): a mission whose
//! model calls would exceed its USD budget is killed at the cap and the
//! result is flagged, so budget-killed missions score as failures.

use hs_loop::*;

const ANSWER: &str = env!("CARGO_BIN_EXE_hs-plugin-answer");
const CHECKER: &str = env!("CARGO_BIN_EXE_hs-plugin-checker");
const BENCHMODEL: &str = env!("CARGO_BIN_EXE_hs-plugin-benchmodel");
const SEQMODEL: &str = env!("CARGO_BIN_EXE_hs-plugin-scripted");
const REPOEXEC: &str = env!("CARGO_BIN_EXE_hs-plugin-repoexec");

#[test]
fn mission_is_killed_at_budget_cap() {
    let dir = tempfile::tempdir().unwrap();
    let log = tempfile::tempdir().unwrap();
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

[[models]]
name = "benchmodel"
command = ["{BENCHMODEL}"]
default = true
"#
        ),
    )
    .unwrap();
    let kernel = hs_kernel::Kernel::load(&config).unwrap();
    // feedback OFF: benchmodel never repairs, so the mission burns calls to
    // the cap. benchmodel reports 900 micro-USD per call; a 5000-micro cap
    // must kill the mission after ~5 calls, long before the 50-step cap
    let mut l = InnerLoop::new(kernel, log.path(), false, 50).unwrap();
    l.set_budget_micros(5_000);
    let r = l.run_mission("task-0").unwrap();
    assert!(!r.passed, "budget-killed mission is a failure");
    assert!(r.budget_killed, "result must flag the budget kill");
    assert!(
        r.model_calls <= 6,
        "killed near the cap, got {}",
        r.model_calls
    );
    assert!(l.total_cost_micros() <= 5_900);
}

#[test]
fn mission_under_budget_runs_normally() {
    // post-gate reality: a mission submits only after verifying, so this
    // fixture scripts the honest flow (repo.exec, blind answer, repair) -
    // the test's subject is budget non-interference, not the gate.
    let dir = tempfile::tempdir().unwrap();
    let log = tempfile::tempdir().unwrap();
    let ws = dir.path().join("ws");
    std::fs::create_dir_all(&ws).unwrap();
    std::fs::write(ws.join("code.txt"), "broken\n").unwrap();
    let cmds: [&[&str]; 3] = [
        &["init", "-q"],
        &["add", "."],
        &[
            "-c",
            "user.email=t@t",
            "-c",
            "user.name=t",
            "commit",
            "-qm",
            "init",
        ],
    ];
    for args in cmds {
        let st = std::process::Command::new("git")
            .args(args)
            .current_dir(&ws)
            .status()
            .unwrap();
        assert!(st.success());
    }
    // FIXME: Audit that the environment access only happens in single-threaded code.
    unsafe { std::env::set_var("HS_SWE_WORKSPACE", &ws) };
    let answer = log.path().join("work").join("task-0").join("answer.txt");
    let diff = "```diff\n--- a/code.txt\n+++ b/code.txt\n@@ -1 +1 @@\n-broken\n+fixed\n```";
    let script = dir.path().join("script.jsonl");
    std::fs::write(
        &script,
        format!(
            "{{\"tool\":\"repo.exec\",\"args\":{{\"command\":\"cat code.txt\",\"diff\":\"{}\"}}}}\n{{\"tool\":\"answer.write\",\"args\":{{\"path\":\"{}\",\"content\":\"alpha\"}}}}\n{{\"tool\":\"answer.write\",\"args\":{{\"path\":\"{}\",\"content\":\"TOKEN-0-SECRET\"}}}}",
            diff.replace('\n', "\\n"),
            answer.display(),
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
    let mut l = InnerLoop::new(kernel, log.path(), true, 6).unwrap();
    l.set_budget_micros(10_000_000); // $10: never fires
    let r = l.run_mission("task-0").unwrap();
    assert!(r.passed, "{r:?}");
    assert!(!r.budget_killed);
    // FIXME: Audit that the environment access only happens in single-threaded code.
    unsafe { std::env::remove_var("HS_SWE_WORKSPACE") };
    // FIXME: Audit that the environment access only happens in single-threaded code.
    unsafe { std::env::remove_var("HS_SEQMODEL_SCRIPT") };
}
