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
    assert_eq!(result["steps"], 3, "read via repo.read, verify via repo.exec, then write (hard answer gate)");
}

/// Seam: repo.exec through the real driver/kernel/loop path (parent bar: no
/// unit-test-only components). sweexec writes a corrupt patch, pre-flights it
/// with repo.exec (must come back applied=false with the git error), then
/// writes the gold patch and passes. Asserts on the event stream itself.
#[test]
fn driver_mission_preflights_with_repoexec() {
    let dir = tempfile::tempdir().unwrap();
    let run_dir = dir.path().join("run");
    let ws = run_dir.join("ws");
    std::fs::create_dir_all(&ws).unwrap();
    std::fs::write(ws.join("code.txt"), "broken\n").unwrap();
    std::fs::write(ws.join("check.sh"), "#!/bin/sh\ngrep -q '^fixed$' code.txt\n").unwrap();
    let git = |args: &[&str]| {
        assert!(Command::new("git").args(args).current_dir(&ws).status().unwrap().success());
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
            "instance_id": "fixture__git-3",
            "problem_statement": "code.txt must contain the word fixed",
            "fail_to_pass": ["sh check.sh"],
        }))
        .unwrap(),
    )
    .unwrap();

    let out = Command::new(DRIVER)
        .args([
            "--instance", &dir.path().join("instance.json").display().to_string(),
            "--model", "sweexec",
            "--feedback", "on",
            "--budget-micros", "100000",
            "--max-steps", "8",
            "--run-dir", &run_dir.display().to_string(),
        ])
        .env("HS_SWE_WORKSPACE", &ws)
        .env("HS_SWE_F2P", "sh check.sh")
        .env("HS_SWE_P2P", "")
        .env("HS_SWE_EXEC_ALLOW", "sh check.sh")
        .env("HS_SWE_GOLD_PATCH_FILE", dir.path().join("gold.patch"))
        .output()
        .unwrap();
    assert!(out.status.success(), "driver: {}", String::from_utf8_lossy(&out.stderr));
    let result: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(run_dir.join("result.json")).unwrap())
            .unwrap();
    assert_eq!(result["passed"], true, "mission must pass after the pre-flight repair: {result}");

    // the stream proves the real path: a repo.exec ToolCall whose result is
    // the free apply-error feedback (corrupt patch), on the audit log
    let streams_dir = run_dir.join("log").join("streams");
    let sid = std::fs::read_dir(&streams_dir).unwrap().next().unwrap().unwrap();
    let sid = uuid::Uuid::parse_str(sid.file_name().to_str().unwrap()).unwrap();
    let reader = hs_log::StreamReader::open(&run_dir.join("log"), sid).unwrap();
    let events = reader.events().unwrap();
    let exec_call = events.iter().find_map(|e| {
        let p = String::from_utf8_lossy(&reader.resolve_payload(e).unwrap()).to_string();
        (p.contains("\"plugin\":\"repo.exec\"")).then_some(p)
    }).expect("a repo.exec ToolCall event must be on the stream");
    assert!(exec_call.contains("\"applied\":false"), "corrupt patch pre-flight feedback: {exec_call}");
    assert!(exec_call.contains("apply_error"), "names the git error: {exec_call}");
    // and the live workspace was never mutated by the exec pre-flight
    // (final state carries the gold patch applied by the CHECKER, post-pass)
    assert_eq!(std::fs::read_to_string(ws.join("code.txt")).unwrap(), "fixed\n");
}

/// Seam: an MCP-discovered tool callable through the real driver/kernel/loop
/// (docs/mcp-adapter-gate.md). HS_MCP_SERVERS points at a [[mcp_servers]]
/// TOML with the in-tree fixture server; the driver discovers its tools via
/// the bridge, registers mcp.fixture.echo in the generated hairspring.toml,
/// and the scripted swemcp model calls it. Asserts on the event stream.
#[test]
fn driver_mission_calls_mcp_tool() {
    let dir = tempfile::tempdir().unwrap();
    let run_dir = dir.path().join("run");
    let ws = run_dir.join("ws");
    std::fs::create_dir_all(&ws).unwrap();
    std::fs::write(ws.join("code.txt"), "broken\n").unwrap();
    std::fs::write(ws.join("check.sh"), "#!/bin/sh\ngrep -q '^fixed$' code.txt\n").unwrap();
    let git = |args: &[&str]| {
        assert!(Command::new("git").args(args).current_dir(&ws).status().unwrap().success());
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
            "instance_id": "fixture__git-4",
            "problem_statement": "code.txt must contain the word fixed",
            "fail_to_pass": ["sh check.sh"],
        }))
        .unwrap(),
    )
    .unwrap();
    let servers = dir.path().join("mcp_servers.toml");
    std::fs::write(
        &servers,
        format!(
            "[[mcp_servers]]\nname = \"fixture\"\ncommand = [\"{}\"]\nallowed_roots = [\"{}\"]\n",
            env!("CARGO_BIN_EXE_hs-mcp-fixture"),
            ws.display()
        ),
    )
    .unwrap();

    let out = Command::new(DRIVER)
        .args([
            "--instance", &dir.path().join("instance.json").display().to_string(),
            "--model", "swemcp",
            "--feedback", "on",
            "--budget-micros", "100000",
            "--max-steps", "8",
            "--run-dir", &run_dir.display().to_string(),
        ])
        .env("HS_SWE_WORKSPACE", &ws)
        .env("HS_SWE_F2P", "sh check.sh")
        .env("HS_SWE_P2P", "")
        .env("HS_MCP_SERVERS", &servers)
        .env("HS_SWE_GOLD_PATCH_FILE", dir.path().join("gold.patch"))
        .output()
        .unwrap();
    assert!(out.status.success(), "driver: {}", String::from_utf8_lossy(&out.stderr));
    // graft-experiment finding (2026-09-05): MCP tools registered in the
    // kernel but absent from the mission prompt are invisible to the model -
    // run B never called one. The prompt must enumerate them.
    let prompt = std::fs::read_to_string(run_dir.join("mission_prompt.txt")).unwrap();
    assert!(
        prompt.contains("mcp.fixture.echo"),
        "mission prompt must enumerate registered MCP tools; got none of mcp.fixture.*"
    );
    let result: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(run_dir.join("result.json")).unwrap())
            .unwrap();
    assert_eq!(result["passed"], true, "mission passes: {result}");

    // the generated config registered the discovered tool
    let cfg = std::fs::read_to_string(run_dir.join("hairspring.toml")).unwrap();
    assert!(cfg.contains("mcp.fixture.echo"), "namespaced tool registered: {cfg}");

    // the stream proves the real path: an mcp.fixture.echo ToolCall whose
    // result carries the echo payload
    let streams_dir = run_dir.join("log").join("streams");
    let sid = std::fs::read_dir(&streams_dir).unwrap().next().unwrap().unwrap();
    let sid = uuid::Uuid::parse_str(sid.file_name().to_str().unwrap()).unwrap();
    let reader = hs_log::StreamReader::open(&run_dir.join("log"), sid).unwrap();
    let events = reader.events().unwrap();
    let mcp_call = events.iter().find_map(|e| {
        let p = String::from_utf8_lossy(&reader.resolve_payload(e).unwrap()).to_string();
        // must be the SUCCESS event: an error payload would also contain the
        // args text, which previously let a dead MCP path pass this test
        (p.contains("\"plugin\":\"mcp.fixture.echo\"") && p.contains("\"result\"") && !p.contains("\"error\"")).then_some(p)
    }).expect("a successful mcp.fixture.echo ToolCall event must be on the stream");
    assert!(mcp_call.contains("hello-via-mcp"), "echo payload on stream: {mcp_call}");
}

/// Seam: the D5 tools (edit.apply, notes.scratch) are wired into every SWE
/// mission: registered in the generated hairspring.toml, callable through
/// the real kernel/loop, with notes persisted at log_root/work/<iid>/.
#[test]
fn driver_mission_uses_d5_tools() {
    let dir = tempfile::tempdir().unwrap();
    let run_dir = dir.path().join("run");
    let ws = run_dir.join("ws");
    std::fs::create_dir_all(&ws).unwrap();
    std::fs::write(ws.join("code.txt"), "broken\n").unwrap();
    std::fs::write(ws.join("check.sh"), "#!/bin/sh\ngrep -q '^fixed$' code.txt\n").unwrap();
    let git = |args: &[&str]| {
        assert!(Command::new("git").args(args).current_dir(&ws).status().unwrap().success());
    };
    git(&["init", "-q"]);
    git(&["config", "user.email", "t@t"]);
    git(&["config", "user.name", "t"]);
    git(&["add", "-A"]);
    git(&["commit", "-qm", "base"]);
    let diff = "--- a/code.txt\n+++ b/code.txt\n@@ -1 +1 @@\n-broken\n+fixed\n";
    std::fs::write(
        dir.path().join("instance.json"),
        serde_json::to_string(&serde_json::json!({
            "instance_id": "fixture__git-5",
            "problem_statement": "code.txt must contain the word fixed",
            "fail_to_pass": ["sh check.sh"],
        }))
        .unwrap(),
    )
    .unwrap();
    let answer_path = run_dir.join("log").join("work").join("fixture__git-5").join("answer.txt");
    let script = [
        serde_json::json!({"tool":"notes.scratch","args":{"op":"write","content":"hypothesis: code.txt holds the wrong word\n"}}).to_string(),
        serde_json::json!({"tool":"notes.scratch","args":{"op":"read"}}).to_string(),
        serde_json::json!({"tool":"edit.apply","args":{"diff":diff}}).to_string(),
        serde_json::json!({"tool":"answer.write","args":{"path":answer_path.display().to_string(),"content":format!("```diff\n{diff}```")}}).to_string(),
    ];
    std::fs::write(dir.path().join("script.jsonl"), script.join("\n")).unwrap();

    let out = Command::new(DRIVER)
        .args([
            "--instance", &dir.path().join("instance.json").display().to_string(),
            "--model", "scripted",
            "--feedback", "on",
            "--budget-micros", "100000",
            "--max-steps", "8",
            "--run-dir", &run_dir.display().to_string(),
        ])
        .env("HS_SWE_WORKSPACE", &ws)
        .env("HS_SWE_F2P", "sh check.sh")
        .env("HS_SWE_P2P", "")
        .env("HS_SEQMODEL_SCRIPT", dir.path().join("script.jsonl"))
        .output()
        .unwrap();
    assert!(out.status.success(), "driver: {}", String::from_utf8_lossy(&out.stderr));
    let result: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(run_dir.join("result.json")).unwrap())
            .unwrap();
    assert_eq!(result["passed"], true, "mission passes: {result}");

    // both D5 tools registered in the generated config
    let cfg = std::fs::read_to_string(run_dir.join("hairspring.toml")).unwrap();
    assert!(cfg.contains("edit.apply"), "edit.apply registered: {cfg}");
    assert!(cfg.contains("notes.scratch"), "notes.scratch registered: {cfg}");

    // notes persisted at the mission work dir
    let notes = run_dir.join("log").join("work").join("fixture__git-5").join("notes.md");
    assert!(std::fs::read_to_string(&notes).unwrap().contains("hypothesis: code.txt holds the wrong word"),
        "notes file at {}", notes.display());

    // the stream proves both tools really ran: notes read returned the
    // content, edit.apply returned a cumulative diff
    let streams_dir = run_dir.join("log").join("streams");
    let sid = std::fs::read_dir(&streams_dir).unwrap().next().unwrap().unwrap();
    let sid = uuid::Uuid::parse_str(sid.file_name().to_str().unwrap()).unwrap();
    let reader = hs_log::StreamReader::open(&run_dir.join("log"), sid).unwrap();
    let events = reader.events().unwrap();
    let payloads: Vec<String> = events.iter()
        .map(|e| String::from_utf8_lossy(&reader.resolve_payload(e).unwrap()).to_string())
        .collect();
    let notes_call = payloads.iter().find(|p| p.contains("\"plugin\":\"notes.scratch\"") && p.contains("\"content\"") && p.contains("hypothesis"))
        .expect("a notes.scratch result carrying the note must be on the stream");
    assert!(notes_call.contains("\"ok\":true"), "{notes_call}");
    let edit_call = payloads.iter().find(|p| p.contains("\"plugin\":\"edit.apply\"") && p.contains("cumulative_diff"))
        .expect("an edit.apply result with cumulative_diff must be on the stream");
    assert!(edit_call.contains("+fixed"), "cumulative diff carries the patch: {edit_call}");
}
