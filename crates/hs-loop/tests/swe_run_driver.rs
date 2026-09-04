//! hs-swe-run driver E2E (seam gap C2/C3, red 2026-09-04): the mission
//! DRIVER binary - arg parsing, config synthesis, repo layout, prompt
//! assembly, result.json, model_patch.diff, ledger.csv - had zero coverage;
//! every one of those lines only ever ran live. This drives the real binary
//! end to end on a fixture repo with the scripted swemodel + real swecheck
//! (git apply + check command): the full assembled mission seam.
use std::process::Command;

const DRIVER: &str = env!("CARGO_BIN_EXE_hs-swe-run");

#[test]
fn driver_runs_a_full_mission_and_writes_all_artifacts() {
    let dir = tempfile::tempdir().unwrap();
    let run_dir = dir.path().join("run");
    let ws = run_dir.join("ws");
    std::fs::create_dir_all(&ws).unwrap();
    std::fs::write(ws.join("code.txt"), "broken\n").unwrap();
    std::fs::write(
        ws.join("check.sh"),
        "#!/bin/sh\ngrep -q '^fixed$' code.txt\n",
    )
    .unwrap();
    let git = |args: &[&str]| {
        assert!(Command::new("git")
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

    std::fs::write(
        dir.path().join("gold.patch"),
        "--- a/code.txt\n+++ b/code.txt\n@@ -1 +1 @@\n-broken\n+fixed\n",
    )
    .unwrap();
    std::fs::write(
        dir.path().join("instance.json"),
        serde_json::to_string(&serde_json::json!({
            "instance_id": "fixture__git-1",
            "problem_statement": "code.txt must contain the word fixed",
            "fail_to_pass": ["sh check.sh"],
        }))
        .unwrap(),
    )
    .unwrap();

    let out = Command::new(DRIVER)
        .args([
            "--instance",
            &dir.path().join("instance.json").display().to_string(),
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
        ])
        .env("HS_SWE_WORKSPACE", &ws)
        .env("HS_SWE_F2P", "sh check.sh")
        .env("HS_SWE_P2P", "")
        .env("HS_SWE_GOLD_PATCH_FILE", dir.path().join("gold.patch"))
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "driver exited {:?}: {}",
        out.status,
        String::from_utf8_lossy(&out.stderr)
    );

    let result: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(run_dir.join("result.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(result["instance_id"], "fixture__git-1");
    assert_eq!(result["passed"], true, "fixture mission must pass");
    assert_eq!(result["budget_killed"], false);
    assert!(result["steps"].as_u64().unwrap() <= 8);

    // the final patch is extracted and written
    let patch = std::fs::read_to_string(run_dir.join("model_patch.diff")).unwrap();
    assert!(patch.contains("+fixed"), "patch content: {patch}");

    // the ledger row is appended
    let ledger = std::fs::read_to_string(run_dir.join("ledger.txt")).unwrap();
    assert!(
        ledger.contains("swemodel,system,fixture__git-1,true,"),
        "ledger: {ledger}"
    );

    // config synthesis + prompt assembly landed on disk
    assert!(run_dir.join("hairspring.toml").exists());
    assert!(run_dir.join("mission_prompt.txt").exists());

    // workspace carries the applied patch
    assert_eq!(std::fs::read_to_string(ws.join("code.txt")).unwrap(), "fixed\n");
}

/// Seam C3: the driver-synthesized config wires repo.read/repo.search
/// through the kernel - a model that READS via those tools before writing
/// proves the assembled tool path, not just the write/check path.
#[test]
fn driver_mission_uses_repotools_through_synthesized_config() {
    let dir = tempfile::tempdir().unwrap();
    let run_dir = dir.path().join("run");
    let ws = run_dir.join("ws");
    std::fs::create_dir_all(&ws).unwrap();
    std::fs::write(ws.join("code.txt"), "broken\n").unwrap();
    std::fs::write(
        ws.join("check.sh"),
        "#!/bin/sh\ngrep -q '^fixed$' code.txt\n",
    )
    .unwrap();
    let git = |args: &[&str]| {
        assert!(Command::new("git")
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
    std::fs::write(
        dir.path().join("gold.patch"),
        "--- a/code.txt\n+++ b/code.txt\n@@ -1 +1 @@\n-broken\n+fixed\n",
    )
    .unwrap();
    std::fs::write(
        dir.path().join("instance.json"),
        serde_json::to_string(&serde_json::json!({
            "instance_id": "fixture__git-2",
            "problem_statement": "code.txt must contain the word fixed",
            "fail_to_pass": ["sh check.sh"],
        }))
        .unwrap(),
    )
    .unwrap();

    let out = Command::new(DRIVER)
        .args([
            "--instance",
            &dir.path().join("instance.json").display().to_string(),
            "--model",
            "swereader",
            "--feedback",
            "on",
            "--budget-micros",
            "100000",
            "--max-steps",
            "8",
            "--run-dir",
            &run_dir.display().to_string(),
        ])
        .env("HS_SWE_WORKSPACE", &ws)
        .env("HS_SWE_F2P", "sh check.sh")
        .env("HS_SWE_P2P", "")
        .env("HS_SWE_GOLD_PATCH_FILE", dir.path().join("gold.patch"))
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "driver exited {:?}: {}",
        out.status,
        String::from_utf8_lossy(&out.stderr)
    );
    let result: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(run_dir.join("result.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(result["passed"], true);
    assert_eq!(result["steps"], 2, "read via repo.read, then write");
}
