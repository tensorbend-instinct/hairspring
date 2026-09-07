//! RED acceptance gates for BLIND completion mode (Eric 2026-09-07: "We want
//! it proved on no fail to pass that's cheating"; parent: the ground-truth
//! FAIL_TO_PASS must never enter the mission - prompt, checker, verifier, or
//! any stream payload the agent can see - and grades after the fact only).
//!
//! B1 blind mission: completes on the agent's own declared checks, no
//!    ground-truth string anywhere agent-visible, patch carries no harness
//!    machinery, and grade.json records the external verdict.
//! B2 blind mission whose own declared checks fail does NOT pass; feedback
//!    names the failing command; external grade records false.

use std::process::Command;

const DRIVER: &str = env!("CARGO_BIN_EXE_hs-swe-run");

fn git(ws: &std::path::Path, args: &[&str]) {
    assert!(Command::new("git")
        .args(args)
        .current_dir(ws)
        .status()
        .unwrap()
        .success());
}

fn fixture(check_script: &str, dir: &std::path::Path) -> (std::path::PathBuf, std::path::PathBuf) {
    let run_dir = dir.join("run");
    let ws = run_dir.join("ws");
    std::fs::create_dir_all(&ws).unwrap();
    std::fs::write(ws.join("code.txt"), "broken\n").unwrap();
    std::fs::write(ws.join("check.sh"), check_script).unwrap();
    git(&ws, &["init", "-q"]);
    git(&ws, &["config", "user.email", "t@t"]);
    git(&ws, &["config", "user.name", "t"]);
    git(&ws, &["add", "-A"]);
    git(&ws, &["commit", "-qm", "base"]);
    std::fs::write(
        dir.join("gold.patch"),
        "--- a/code.txt\n+++ b/code.txt\n@@ -1 +1 @@\n-broken\n+fixed\n",
    )
    .unwrap();
    (run_dir, ws)
}

fn write_instance(dir: &std::path::Path) -> std::path::PathBuf {
    let p = dir.join("instance.json");
    std::fs::write(
        &p,
        serde_json::to_string(&serde_json::json!({
            "instance_id": "fixture__blind-1",
            "problem_statement": "code.txt must contain the word fixed",
            "fail_to_pass": ["SECRET_F2P_MARKER"],
        }))
        .unwrap(),
    )
    .unwrap();
    p
}

fn all_stream_payloads(run_dir: &std::path::Path) -> String {
    let log_root = run_dir.join("log");
    let streams = log_root.join("streams");
    let mut all = String::new();
    for e in std::fs::read_dir(&streams).unwrap() {
        let sid = uuid::Uuid::parse_str(&e.unwrap().file_name().to_string_lossy()).unwrap();
        let reader = hs_log::StreamReader::open(&log_root, sid).unwrap();
        for ev in reader.events().unwrap() {
            if let Ok(b) = reader.resolve_payload(&ev) {
                all.push_str(&String::from_utf8_lossy(&b));
                all.push('\n');
            }
        }
    }
    all
}

#[test]
fn b1_blind_mission_never_sees_f2p_and_grades_externally() {
    let dir = tempfile::tempdir().unwrap();
    let (run_dir, ws) = fixture("#!/bin/sh\ngrep -q '^fixed$' code.txt\n", dir.path());
    let inst = write_instance(dir.path());

    let out = Command::new(DRIVER)
        .args([
            "--instance",
            &inst.display().to_string(),
            "--model",
            "swemodel",
            "--feedback",
            "on",
            "--budget-micros",
            "100000",
            "--max-steps",
            "8",
            "--run-dir",
            &run_dir.display().to_string(),
            "--mode",
            "blind",
            "--grade-cmd",
            "sh check.sh",
        ])
        .env("HS_SWE_WORKSPACE", &ws)
        .env("HS_SWE_GOLD_PATCH_FILE", dir.path().join("gold.patch"))
        // deliberately NO HS_SWE_F2P: blind mode must not need it
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "driver exited {:?}: {}",
        out.status,
        String::from_utf8_lossy(&out.stderr)
    );

    let result: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(run_dir.join("result.json")).unwrap())
            .unwrap();
    assert_eq!(result["mode"], "blind", "result records the mode: {result}");
    assert_eq!(
        result["passed"], true,
        "blind mission passes on the agent's own checks: {result}"
    );

    // the prompt teaches self-verification and carries NO ground truth
    let prompt = std::fs::read_to_string(run_dir.join("mission_prompt.txt")).unwrap();
    assert!(
        !prompt.contains("SECRET_F2P_MARKER"),
        "prompt leaks the ground-truth test id"
    );
    assert!(
        !prompt.contains("FAIL_TO_PASS"),
        "prompt still speaks the f2p protocol: {prompt}"
    );
    assert!(
        prompt.contains(".hs/checks"),
        "blind prompt must teach the self-check channel: {prompt}"
    );

    // no event payload on ANY stream (agent + verifier) names the marker
    let payloads = all_stream_payloads(&run_dir);
    assert!(
        !payloads.contains("SECRET_F2P_MARKER"),
        "ground truth leaked into a mission stream"
    );

    // the submitted patch is the fix and nothing else: the agent's checks
    // declaration is harness machinery, never patch content
    let patch = std::fs::read_to_string(run_dir.join("model_patch.diff")).unwrap();
    assert!(patch.contains("+fixed"), "patch content: {patch}");
    assert!(
        !patch.contains(".hs/checks"),
        "checks declaration leaked into the submitted patch: {patch}"
    );

    // external grading ran AFTER the mission and recorded its own verdict
    let grade: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(run_dir.join("grade.json")).unwrap())
            .unwrap();
    assert_eq!(grade["graded"], true, "grade.json: {grade}");
    assert_eq!(grade["passed"], true, "external grade: {grade}");
}

#[test]
fn b2_blind_mission_with_failing_self_checks_does_not_pass() {
    let dir = tempfile::tempdir().unwrap();
    // the agent's own declared check fails against its fix: no pass, ever
    let (run_dir, ws) = fixture("#!/bin/sh\ngrep -q '^other$' code.txt\n", dir.path());
    let inst = write_instance(dir.path());

    let out = Command::new(DRIVER)
        .args([
            "--instance",
            &inst.display().to_string(),
            "--model",
            "swemodel",
            "--feedback",
            "on",
            "--budget-micros",
            "100000",
            "--max-steps",
            "6",
            "--run-dir",
            &run_dir.display().to_string(),
            "--mode",
            "blind",
            "--grade-cmd",
            "sh check.sh",
        ])
        .env("HS_SWE_WORKSPACE", &ws)
        .env("HS_SWE_GOLD_PATCH_FILE", dir.path().join("gold.patch"))
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "driver exited {:?}: {}",
        out.status,
        String::from_utf8_lossy(&out.stderr)
    );

    let result: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(run_dir.join("result.json")).unwrap())
            .unwrap();
    assert_eq!(
        result["passed"], false,
        "failing self-checks must not pass: {result}"
    );

    // the checker feedback names the failing command for the repair loop
    let payloads = all_stream_payloads(&run_dir);
    assert!(
        payloads.contains("sh check.sh"),
        "checker feedback must name the failing declared command"
    );

    // the external grade agrees: ground truth says the fix is wrong
    let grade: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(run_dir.join("grade.json")).unwrap())
            .unwrap();
    assert_eq!(grade["graded"], true, "grade.json: {grade}");
    assert_eq!(grade["passed"], false, "external grade: {grade}");
}
