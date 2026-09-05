//! GATE 8 BENCHMARK - SWE mission shape, offline integration proof.
//!
//! A full repo+patch mission through the REAL inner loop: model emits a JSON
//! tool call carrying a diff; swecheck applies it in a git workspace and
//! runs the FAIL_TO_PASS command; failure feedback names the failure; the
//! next attempt repairs. Falsifiable: if a no-diff answer can pass, or
//! feedback does not reach the model, the SWE mission shape is broken.

use hs_loop::*;

const ANSWER: &str = env!("CARGO_BIN_EXE_hs-plugin-answer");
const SWECHECK: &str = env!("CARGO_BIN_EXE_hs-plugin-swecheck");
const SWEMODEL: &str = env!("CARGO_BIN_EXE_hs-plugin-swemodel");

fn fixture_workspace(dir: &std::path::Path) -> (std::path::PathBuf, String) {
    let ws = dir.join("ws");
    std::fs::create_dir_all(&ws).unwrap();
    std::fs::write(ws.join("code.txt"), "broken\n").unwrap();
    std::fs::write(
        ws.join("check.sh"),
        "#!/bin/sh\ngrep -q '^fixed$' code.txt\n",
    )
    .unwrap();
    let git = |args: &[&str]| {
        assert!(std::process::Command::new("git")
            .args(args)
            .current_dir(&ws)
            .status()
            .unwrap()
            .success());
    };
    git(&["init", "-q"]);
    git(&["config", "user.email", "t@t"]);
    git(&["config", "user.name", "t"]);
    git(&["add", "-A"]);
    git(&["commit", "-qm", "base"]);
    let gold = "--- a/code.txt\n+++ b/code.txt\n@@ -1 +1 @@\n-broken\n+fixed\n";
    std::fs::write(dir.join("gold.patch"), gold).unwrap();
    (ws, gold.to_string())
}

#[test]
fn swe_mission_repairs_via_feedback_and_passes() {
    let dir = tempfile::tempdir().unwrap();
    let (ws, _gold) = fixture_workspace(dir.path());
    std::env::set_var("HS_SWE_WORKSPACE", &ws);
    std::env::set_var("HS_SWE_F2P", "sh check.sh");
    std::env::set_var("HS_SWE_P2P", "");
    std::env::set_var("HS_SWE_GOLD_PATCH_FILE", dir.path().join("gold.patch"));

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
command = ["{SWECHECK}"]
subjects = ["*"]

[[models]]
name = "swemodel"
command = ["{SWEMODEL}"]
default = true
"#
        ),
    )
    .unwrap();
    let kernel = hs_kernel::Kernel::load(&config).unwrap();
    let log = dir.path().join("log");
    let mut l = InnerLoop::new(kernel, &log, true, 8).unwrap();
    l.set_budget_micros(10_000);
    let prompt = "MISSION fixture__git-1: code.txt must contain the word fixed. \
                  Reply with a JSON tool call answer.write whose content is one fenced unified diff.";
    let r = match l.run_mission_full("fixture__git-1", prompt) {
        Ok(r) => r,
        Err(e) => panic!("mission errored: {e:?}"),
    };
    assert!(r.passed, "SWE mission must pass after feedback repair");
    assert_eq!(r.steps, 2, "prose first, gold patch after feedback");
    assert!(!r.budget_killed);
    // the workspace carries the applied patch
    assert_eq!(std::fs::read_to_string(ws.join("code.txt")).unwrap(), "fixed\n");
}

/// Fix 2 (Eric, 2026-09-05): every mission prompt opens with an orientation
/// brief - a map, not a manual. The model must KNOW it has a real machine:
/// root, writable system roots, network on, which package managers exist.
/// ab2: missions burned steps probing "can I even run pip?" instead of
/// working, because the prompt described a toy jail that no longer exists.
#[test]
fn mission_prompt_opens_with_machine_orientation() {
    let args = hs_loop::sweprompt::PromptArgs {
        ws: "/tmp/ws".into(),
        problem_statement: "bug".into(),
        fail_to_pass: vec!["pytest t -x".into()],
        repo_layout: "src/main.rs\n".into(),
        nudge: String::new(),
        answer_path: "/tmp/answer.txt".into(),
        orientation: hs_loop::sweprompt::probe_orientation(),
    };
    let prompt = hs_loop::sweprompt::build_mission_prompt(None, &args);
    assert!(prompt.contains("MACHINE:"), "orientation block present: {}", &prompt[..prompt.len().min(600)]);
    assert!(prompt.contains("Network: ON"), "network state stated: {prompt}");
    assert!(prompt.contains("root"), "identity stated: {prompt}");
    assert!(prompt.contains("Detected tooling:"), "probed tooling line: {prompt}");
    assert!(prompt.contains("python3"), "python3 detected on this box: {prompt}");
    assert!(!prompt.contains("no network, no host fs"), "stale jail description removed: {prompt}");
}

/// Verifier-policy item 1 (Eric, 2026-09-05): the mission template carries
/// Grok Build's three work-policy disciplines - they attack the exact ab2
/// failure (quiet stalling, unverified claims, offers instead of action).
#[test]
fn mission_prompt_carries_work_policy_discipline() {
    let args = hs_loop::sweprompt::PromptArgs {
        ws: "/tmp/ws".into(),
        problem_statement: "bug".into(),
        fail_to_pass: vec!["pytest t -x".into()],
        repo_layout: "src/main.rs\n".into(),
        nudge: String::new(),
        answer_path: "/tmp/answer.txt".into(),
        orientation: String::new(),
    };
    let prompt = hs_loop::sweprompt::build_mission_prompt(None, &args);
    assert!(prompt.contains("WORK POLICY:"), "policy block present: {prompt}");
    assert!(prompt.contains("only when tool output supports the claim"),
        "claim discipline: {prompt}");
    assert!(prompt.contains("say so plainly rather than quietly dropping it"),
        "blocked discipline: {prompt}");
    assert!(prompt.contains("current step instead of ending with an offer"),
        "action-now discipline: {prompt}");
}
