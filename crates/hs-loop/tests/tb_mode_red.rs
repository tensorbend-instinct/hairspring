//! RED acceptance gates for TERMINAL-BENCH mode (Eric 2026-09-07: "let's give
//! it terminal-bench-4 tests"). TB tasks grade the LIVE container state with
//! hidden official tests that run only after the agent finishes (Harbor
//! separate-verifier mode) - so the mission shape differs from SWE: the agent
//! works directly on the machine (no candidate, no patch), declares its own
//! checks in .hs/checks (blind stop authority, same as swe blind mode), and
//! finishes with answer.submit. Ground truth never enters the mission.
//!
//! T1 term.exec runs directly and state PERSISTS between calls (no sandbox
//!    copy - the container is the sandbox).
//! T2 selfcheck direct mode runs .hs/checks from the live workdir, green only
//!    when every declared command passes; failures name the command.
//! T3 the TB mission prompt teaches .hs/checks + term.exec, carries the task
//!    instruction, and never mentions FAIL_TO_PASS.
//! T4 a full TB mission on the scripted model completes: the agent's term.exec
//!    edits land in the live workdir, selfcheck goes green, result passed.
//! T5 a TB mission whose declared checks fail does NOT pass.

use std::process::Command;

const DRIVER: &str = env!("CARGO_BIN_EXE_hs-tb-run");

fn fixture(dir: &std::path::Path) -> (std::path::PathBuf, std::path::PathBuf) {
    let run_dir = dir.join("run");
    let ws = run_dir.join("ws");
    std::fs::create_dir_all(&ws).unwrap();
    std::fs::write(ws.join("code.txt"), "broken\n").unwrap();
    std::fs::write(ws.join("check.sh"), "#!/bin/sh\ngrep -q '^fixed$' code.txt\n").unwrap();
    (run_dir, ws)
}

fn write_instruction(dir: &std::path::Path) -> std::path::PathBuf {
    let p = dir.join("instruction.md");
    std::fs::write(&p, "Make code.txt contain the word fixed.\n").unwrap();
    p
}

#[test]
fn t1_termexec_direct_and_persistent() {
    let dir = tempfile::tempdir().unwrap();
    let wd = dir.path();
    let r1 = hs_loop::termexec::run(wd, "printf 'hello' > state.txt", 30);
    assert_eq!(r1["exit_code"], 0, "write runs: {r1}");
    let r2 = hs_loop::termexec::run(wd, "cat state.txt", 30);
    assert_eq!(r2["exit_code"], 0);
    assert_eq!(
        r2["stdout"].as_str().unwrap(),
        "hello",
        "state persists across calls (no per-call copy): {r2}"
    );
    let r3 = hs_loop::termexec::run(wd, "exit 3", 30);
    assert_eq!(r3["exit_code"], 3, "exit code surfaces: {r3}");
}

#[test]
fn t2_selfcheck_direct_mode() {
    let dir = tempfile::tempdir().unwrap();
    let ws = dir.path();
    // no candidate worktree at all: direct mode must read the live workdir
    std::fs::create_dir_all(ws.join(".hs")).unwrap();
    std::fs::write(ws.join(".hs/checks"), "true\nfalse\n").unwrap();
    unsafe { std::env::set_var("HS_SELFCHECK_DIRECT", "1") };
    let bad = hs_loop::selfcheck::check(ws);
    assert_eq!(bad["passed"], false, "a failing declared check fails: {bad}");
    assert!(bad["error"].as_str().unwrap().contains("`false`"), "names the failing command: {bad}");
    std::fs::write(ws.join(".hs/checks"), "true\ntrue\n").unwrap();
    let good = hs_loop::selfcheck::check(ws);
    assert_eq!(good["passed"], true, "all-green checks pass: {good}");
    unsafe { std::env::remove_var("HS_SELFCHECK_DIRECT") };
}

#[test]
fn t3_tb_prompt_teaches_own_checks_and_hides_grading() {
    let p = hs_loop::sweprompt::build_tb_mission_prompt(&hs_loop::sweprompt::TbPromptArgs {
        workdir: "/app".into(),
        instruction: "Do the thing.".into(),
        answer_path: "/tmp/answer.txt".into(),
    });
    for want in [".hs/checks", "term.exec", "Do the thing.", "ANSWER_PATH: /tmp/answer.txt"] {
        assert!(p.contains(want), "prompt missing {want:?}");
    }
    assert!(!p.contains("FAIL_TO_PASS"), "no f2p language in tb prompt");
}

#[test]
fn t4_tb_mission_completes_on_own_checks() {
    let dir = tempfile::tempdir().unwrap();
    let (run_dir, ws) = fixture(dir.path());
    let inst = write_instruction(dir.path());
    let out = Command::new(DRIVER)
        .args([
            "--instruction", &inst.display().to_string(),
            "--task-id", "fixture__tb-1",
            "--model", "swemodel",
            "--feedback", "on",
            "--budget-micros", "100000",
            "--max-steps", "8",
            "--run-dir", &run_dir.display().to_string(),
            "--workdir", &ws.display().to_string(),
        ])
        .output()
        .unwrap();
    assert!(out.status.success(), "driver: {}", String::from_utf8_lossy(&out.stderr));
    let result: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(run_dir.join("result.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(result["mode"], "tb-blind");
    assert_eq!(result["passed"], true, "mission passes on its own checks: {result}");
    assert_eq!(
        std::fs::read_to_string(ws.join("code.txt")).unwrap(),
        "fixed\n",
        "term.exec edits land in the LIVE workdir"
    );
    assert_eq!(
        std::fs::read_to_string(ws.join(".hs/checks")).unwrap().trim(),
        "sh check.sh",
        "the agent declared its own checks"
    );
    let prompt = std::fs::read_to_string(run_dir.join("mission_prompt.txt")).unwrap();
    assert!(prompt.contains("Make code.txt contain the word fixed."));
    assert!(!prompt.contains("FAIL_TO_PASS"));
}

#[test]
fn t5_tb_mission_with_failing_checks_does_not_pass() {
    let dir = tempfile::tempdir().unwrap();
    let (run_dir, ws) = fixture(dir.path());
    let inst = write_instruction(dir.path());
    let out = Command::new(DRIVER)
        .args([
            "--instruction", &inst.display().to_string(),
            "--task-id", "fixture__tb-2",
            "--model", "swemodel",
            "--feedback", "on",
            "--budget-micros", "100000",
            "--max-steps", "8",
            "--run-dir", &run_dir.display().to_string(),
            "--workdir", &ws.display().to_string(),
        ])
        .env("HS_SWEMODEL_NOFIX", "1")
        .output()
        .unwrap();
    assert!(out.status.success(), "driver: {}", String::from_utf8_lossy(&out.stderr));
    let result: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(run_dir.join("result.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(result["passed"], false, "failing self-checks cannot pass: {result}");
}

/// T6 (RED): term.exec calls must land in the evidence ledger exactly like
/// repo.exec runs do - in tb mode term.exec is BOTH the edit channel and
/// the only verification channel. Without this the model can never become
/// "verified" and every submission is refuted as unverifiable forever
/// (observed live: smoke run 2026-09-07, 33 calls, ledger empty,
/// VERIFIER REFUTED blocking=unverifiable on every submit).
#[test]
fn t6_term_exec_feeds_evidence_ledger() {
    let mut ledger = hs_loop::ledger::Ledger::default();
    assert!(!ledger.model_verified());
    ledger.apply_tool_call(
        1,
        "term.exec",
        &serde_json::json!({"command": "bash /app/.hs/checks"}),
        &serde_json::json!({"exit_code": 0, "stdout": "ALL_PASS\n", "stderr": "", "timed_out": false}),
    );
    assert!(
        ledger.model_verified(),
        "a successful term.exec run is model verification evidence"
    );
    let rendered = ledger.summary();
    assert!(
        rendered.contains("bash /app/.hs/checks"),
        "ledger must show the term.exec command, got: {rendered}"
    );
    // failing runs are recorded too (audit trail), still count as the model
    // having run its checks
    ledger.apply_tool_call(
        2,
        "term.exec",
        &serde_json::json!({"command": "sh check.sh"}),
        &serde_json::json!({"exit_code": 1, "stdout": "FAIL\n", "stderr": "", "timed_out": false}),
    );
    let rendered = ledger.summary();
    assert!(rendered.contains("sh check.sh"), "got: {rendered}");
}
