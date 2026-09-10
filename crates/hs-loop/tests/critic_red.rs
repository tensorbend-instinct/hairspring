//! RED acceptance gates for the CRITIC self-check standard (Eric 2026-09-07:
//! "try and test it"). The author writes its checks; an INDEPENDENT critic -
//! a fresh model context that never saw the authoring session - gets the
//! original instruction, the declared checks, and shell access, with one
//! directive: REFUTE. A submission passes only when the author's checks are
//! green AND the critic cannot refute it. Fail-closed everywhere: caps,
//! errors, malformed verdicts all mean NOT passed.
//!
//! U1-U6 the refutation loop (scripted model, no network).
//! T7 critic refutes -> mission does NOT pass even with green author checks.
//! T8 critic cannot refute -> mission passes; trace file written.
//! T9 author checks red -> phase-1 failure, critic never consulted.
//! T10 critic-mode mission prompt discloses the critic to the author.

use std::process::Command;

/// The critic runs unprivileged (uid nobody) since the read-only
/// enforcement fix: fixture dirs must be world-traversable like real
/// task workdirs (/app, log work dirs) - tempfile's 0700 default would
/// deny probes for the wrong reason.
fn traversable(dir: &tempfile::TempDir) {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o755)).unwrap();
}

const DRIVER: &str = env!("CARGO_BIN_EXE_hs-tb-run");

fn fixture(dir: &std::path::Path) -> (std::path::PathBuf, std::path::PathBuf) {
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    let run_dir = dir.join("run");
    let ws = run_dir.join("ws");
    std::fs::create_dir_all(&ws).unwrap();
    std::fs::write(ws.join("code.txt"), "broken\n").unwrap();
    std::fs::write(ws.join("check.sh"), "#!/bin/sh\ngrep -q '^fixed$' code.txt\n").unwrap();
    (run_dir, ws)
}

/// U1: a refuted verdict fails the gate and carries the reason.
#[test]
fn u1_refuted_fails_with_reason() {
    let dir = tempfile::tempdir().unwrap();
    let mut m = hs_loop::critic::ScriptedCritic::new(vec![
        hs_loop::critic::CriticReply::Final("{\"refuted\": true, \"reason\": \"re-derived efficiency by back-calculation: got 0.55, task requires 0.96-0.98\"}".into()),
    ]);
    let r = hs_loop::critic::refute(
        dir.path(), "Compute the efficiency.", "cat results.txt",
        &hs_loop::critic::RefuteConfig::default(), &mut m,
    );
    assert!(!r.passed, "refuted must fail: {r:?}");
    assert!(r.reason.contains("0.55"), "reason carried: {r:?}");
}

/// U2: a clean verdict passes.
#[test]
fn u2_clean_verdict_passes() {
    let dir = tempfile::tempdir().unwrap();
    let mut m = hs_loop::critic::ScriptedCritic::new(vec![
        hs_loop::critic::CriticReply::ToolCalls(vec![("c1".into(), "true".into())]),
        hs_loop::critic::CriticReply::Final("{\"refuted\": false, \"reason\": \"re-derived all values by independent method; all requirements tested\"}".into()),
    ]);
    let r = hs_loop::critic::refute(
        dir.path(), "Compute the efficiency.", "cat results.txt",
        &hs_loop::critic::RefuteConfig::default(), &mut m,
    );
    assert!(r.passed, "clean verdict after a real probe passes: {r:?}");
}

/// U3: the critic's shell probes actually RUN on the workdir and the output
/// is fed back to the model.
#[test]
fn u3_tool_calls_execute_and_feed_back() {
    let dir = tempfile::tempdir().unwrap();
    traversable(&dir);
    std::fs::write(dir.path().join("results.txt"), "beta-ok\n").unwrap();
    let mut m = hs_loop::critic::ScriptedCritic::new(vec![
        hs_loop::critic::CriticReply::ToolCalls(vec![("c1".into(), "cat results.txt".into())]),
        hs_loop::critic::CriticReply::Final("{\"refuted\": false, \"reason\": \"value matches\"}".into()),
    ]);
    let r = hs_loop::critic::refute(
        dir.path(), "Write beta-ok to results.txt.", "cat results.txt",
        &hs_loop::critic::RefuteConfig::default(), &mut m,
    );
    assert!(r.passed, "{r:?}");
    let fed = m.seen_tool_results();
    assert!(
        fed.iter().any(|s| s.contains("beta-ok")),
        "tool output fed back to the critic model: {fed:?}"
    );
}

/// U4: step cap is FAIL-CLOSED (a capped critic is not a clean critic).
#[test]
fn u4_step_cap_fails_closed() {
    let dir = tempfile::tempdir().unwrap();
    let mut m = hs_loop::critic::ScriptedCritic::new(vec![
        hs_loop::critic::CriticReply::ToolCalls(vec![("c1".into(), "true".into())]),
        hs_loop::critic::CriticReply::ToolCalls(vec![("c2".into(), "true".into())]),
        hs_loop::critic::CriticReply::ToolCalls(vec![("c3".into(), "true".into())]),
    ]);
    let cfg = hs_loop::critic::RefuteConfig { max_steps: 2, ..Default::default() };
    let r = hs_loop::critic::refute(dir.path(), "Do x.", "true", &cfg, &mut m);
    assert!(!r.passed, "capped critic must fail closed: {r:?}");
    assert!(r.reason.to_lowercase().contains("cap"), "says why: {r:?}");
}

/// U5: a malformed final message is FAIL-CLOSED.
#[test]
fn u5_malformed_verdict_fails_closed() {
    let dir = tempfile::tempdir().unwrap();
    let mut m = hs_loop::critic::ScriptedCritic::new(vec![
        hs_loop::critic::CriticReply::Final("looks fine to me!".into()),
    ]);
    let r = hs_loop::critic::refute(
        dir.path(), "Do x.", "true",
        &hs_loop::critic::RefuteConfig::default(), &mut m,
    );
    assert!(!r.passed, "unparseable verdict fails closed: {r:?}");
}

/// U6: a model error is FAIL-CLOSED.
#[test]
fn u6_model_error_fails_closed() {
    let dir = tempfile::tempdir().unwrap();
    let mut m = hs_loop::critic::ScriptedCritic::failing("http 500");
    let r = hs_loop::critic::refute(
        dir.path(), "Do x.", "true",
        &hs_loop::critic::RefuteConfig::default(), &mut m,
    );
    assert!(!r.passed, "model error fails closed: {r:?}");
    assert!(r.reason.contains("http 500"), "{r:?}");
}

fn run_driver(run_dir: &std::path::Path, ws: &std::path::Path, inst: &std::path::Path, script: &str) -> serde_json::Value {
    let out = Command::new(DRIVER)
        .args([
            "--instruction", &inst.display().to_string(),
            "--task-id", "fixture__critic",
            "--model", "swemodel",
            "--feedback", "on",
            "--critic", "on",
            "--budget-micros", "100000",
            "--max-steps", "10",
            "--run-dir", &run_dir.display().to_string(),
            "--workdir", &ws.display().to_string(),
        ])
        .env("HS_CRITIC_SCRIPT", script)
        .output()
        .unwrap();
    assert!(out.status.success(), "driver: {}", String::from_utf8_lossy(&out.stderr));
    serde_json::from_str(&std::fs::read_to_string(run_dir.join("result.json")).unwrap()).unwrap()
}

/// T7: green author checks + critic refutation = mission does NOT pass.
#[test]
fn t7_critic_refutation_blocks_green_author_checks() {
    let dir = tempfile::tempdir().unwrap();
    let (run_dir, ws) = fixture(dir.path());
    let inst = dir.path().join("instruction.md");
    std::fs::write(&inst, "Make code.txt contain the word fixed.\n").unwrap();
    let result = run_driver(&run_dir, &ws, &inst, "refute:re-derived differently, value wrong");
    assert_eq!(result["passed"], false, "critic refutation blocks: {result}");
    // the refutation reached the author as feedback
    let trace = std::fs::read_to_string(run_dir.join("critic-trace.jsonl")).unwrap();
    assert!(trace.contains("re-derived differently"), "critic reason recorded: {trace}");
    // author still did the work
    assert_eq!(std::fs::read_to_string(ws.join("code.txt")).unwrap(), "fixed\n");
}

/// T8: author checks green + critic clean = mission passes; trace recorded.
#[test]
fn t8_critic_clean_lets_green_mission_pass() {
    let dir = tempfile::tempdir().unwrap();
    let (run_dir, ws) = fixture(dir.path());
    let inst = dir.path().join("instruction.md");
    std::fs::write(&inst, "Make code.txt contain the word fixed.\n").unwrap();
    let script = "tool:cat code.txt|clean";
    let result = run_driver(&run_dir, &ws, &inst, script);
    assert_eq!(result["passed"], true, "clean critic lets it pass: {result}");
    let trace = std::fs::read_to_string(run_dir.join("critic-trace.jsonl")).unwrap();
    assert!(trace.contains("cat code.txt"), "critic probe recorded: {trace}");
    assert!(trace.contains("refuted"), "verdict recorded: {trace}");
}

/// T9: red author checks fail at phase 1 - the critic is never consulted.
#[test]
fn t9_red_author_checks_never_reach_critic() {
    let dir = tempfile::tempdir().unwrap();
    let (run_dir, ws) = fixture(dir.path());
    let inst = dir.path().join("instruction.md");
    std::fs::write(&inst, "Make code.txt contain the word fixed.\n").unwrap();
    let out = Command::new(DRIVER)
        .args([
            "--instruction", &inst.display().to_string(),
            "--task-id", "fixture__critic9",
            "--model", "swemodel",
            "--feedback", "on",
            "--critic", "on",
            "--budget-micros", "100000",
            "--max-steps", "6",
            "--run-dir", &run_dir.display().to_string(),
            "--workdir", &ws.display().to_string(),
        ])
        .env("HS_CRITIC_SCRIPT", "clean")
        .env("HS_SWEMODEL_NOFIX", "1")
        .output()
        .unwrap();
    assert!(out.status.success(), "driver: {}", String::from_utf8_lossy(&out.stderr));
    let result: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(run_dir.join("result.json")).unwrap()).unwrap();
    assert_eq!(result["passed"], false, "{result}");
    assert!(
        !run_dir.join("critic-trace.jsonl").exists(),
        "phase-1 failure means no critic transcript"
    );
}

/// T10: critic-mode prompt discloses the independent critic to the author.
#[test]
fn t10_critic_mode_prompt_discloses_critic() {
    let p = hs_loop::sweprompt::build_tb_mission_prompt_critic(&hs_loop::sweprompt::TbPromptArgs {
        workdir: "/app".into(),
        instruction: "Do the thing.".into(),
        answer_path: "/tmp/answer.txt".into(),
        mcp_tools: String::new(),
    });
    let pl = p.to_lowercase();
    assert!(pl.contains("critic"), "names the critic");
    assert!(pl.contains("refute"), "names refutation");
    assert!(pl.contains("different method"), "names independent re-derivation");
    assert!(!p.contains("FAIL_TO_PASS"));
}

/// U7: critic model selection. Default `DeepSeek`; "glm" selects the z.ai
/// provider (cross-family critic); unknown names are an error (fail-closed
/// at the gate, never a silent fallback).
#[test]
fn u7_critic_model_selection() {
    let d = hs_loop::critic::provider_for("deepseek").unwrap();
    assert!(d.default_base_url.contains("deepseek"), "{d:?}");
    let g = hs_loop::critic::provider_for("glm").unwrap();
    assert!(g.default_base_url.contains("z.ai"), "{g:?}");
    assert!(g.default_model.contains("glm"), "{g:?}");
    assert!(hs_loop::critic::provider_for("bogus").is_err());
}

/// U8: a CLEAN verdict with zero machine probes is fail-closed - the critic
/// must have actually touched the machine before clearing a submission.
#[test]
fn u8_clean_verdict_without_probes_fails_closed() {
    let dir = tempfile::tempdir().unwrap();
    let mut m = hs_loop::critic::ScriptedCritic::new(vec![
        hs_loop::critic::CriticReply::Final("{\"refuted\": false, \"reason\": \"trust me\"}".into()),
    ]);
    let r = hs_loop::critic::refute(
        dir.path(), "Do x.", "true",
        &hs_loop::critic::RefuteConfig::default(), &mut m,
    );
    assert!(!r.passed, "probe-less clean verdict must fail closed: {r:?}");
    assert!(r.reason.to_lowercase().contains("probe"), "says why: {r:?}");
}

/// U9: a REFUTED verdict needs no probes to count - refutation is the
/// fail-closed direction already.
#[test]
fn u9_refuted_verdict_without_probes_still_fails() {
    let dir = tempfile::tempdir().unwrap();
    let mut m = hs_loop::critic::ScriptedCritic::new(vec![
        hs_loop::critic::CriticReply::Final("{\"refuted\": true, \"reason\": \"results.txt missing\"}".into()),
    ]);
    let r = hs_loop::critic::refute(
        dir.path(), "Do x.", "true",
        &hs_loop::critic::RefuteConfig::default(), &mut m,
    );
    assert!(!r.passed, "{r:?}");
    assert!(r.reason.contains("results.txt missing"), "{r:?}");
}

/// T11: cross-model wiring - the scripted seam still wins (no network in
/// tests), and an unknown critic model fails the gate closed.
#[test]
fn t11_critic_model_env_selection() {
    // unknown model, no script: the gate must fail closed
    let dir = tempfile::tempdir().unwrap();
    let (run_dir, ws) = fixture(dir.path());
    let inst = dir.path().join("instruction.md");
    std::fs::write(&inst, "Make code.txt contain the word fixed.\n").unwrap();
    let out = Command::new(DRIVER)
        .args([
            "--instruction", &inst.display().to_string(),
            "--task-id", "fixture__critic11",
            "--model", "swemodel",
            "--feedback", "on",
            "--critic", "on",
            "--budget-micros", "100000",
            "--max-steps", "6",
            "--run-dir", &run_dir.display().to_string(),
            "--workdir", &ws.display().to_string(),
        ])
        .env("HS_CRITIC_MODEL", "bogus")
        .output()
        .unwrap();
    assert!(out.status.success(), "driver: {}", String::from_utf8_lossy(&out.stderr));
    let result: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(run_dir.join("result.json")).unwrap()).unwrap();
    assert_eq!(result["passed"], false, "unknown critic model must fail closed: {result}");

    // glm selected + script seam: the script wins (no network), mission passes
    let dir2 = tempfile::tempdir().unwrap();
    let (run_dir2, ws2) = fixture(dir2.path());
    let result2 = run_driver(&run_dir2, &ws2, &inst, "tool:cat code.txt|clean");
    assert_eq!(result2["passed"], true, "{result2}");
}

// BURN-DOWN (spec-cut: critic precision) - RED first.
// The DISC matrix measured 13 fail-closed false vetoes vs 1 real catch;
// "verdict unparseable" was the top cause. One malformed reply must earn
// a schema-tightening retry, not an instant fail-closed.
#[test]
fn critic_parse_retry_recovers_verdict() {
    let dir = tempfile::tempdir().unwrap();
    let ws = dir.path().join("ws");
    std::fs::create_dir_all(&ws).unwrap();
    let mut m = hs_loop::critic::ScriptedCritic::new(vec![
        hs_loop::critic::CriticReply::Final("let me think out loud with no json at all".into()),
        hs_loop::critic::CriticReply::ToolCalls(vec![("c1".into(), "true".into())]),
        hs_loop::critic::CriticReply::Final("{\"refuted\": false, \"reason\": \"re-derived by independent method\"}".into()),
    ]);
    let r = hs_loop::critic::refute(
        &ws,
        "task",
        "checks",
        &hs_loop::critic::RefuteConfig::default(),
        &mut m,
    );
    assert!(r.passed, "one malformed verdict must earn a retry, not a fail-closed: {}", r.reason);
    assert!(
        r.trace.iter().any(|t| t["kind"] == "verdict_retry"),
        "the retry must be visible in the trace: {:?}", r.trace
    );
}

#[test]
fn critic_parse_retry_twice_unparseable_fails_closed() {
    let dir = tempfile::tempdir().unwrap();
    let ws = dir.path().join("ws");
    std::fs::create_dir_all(&ws).unwrap();
    let mut m = hs_loop::critic::ScriptedCritic::new(vec![
        hs_loop::critic::CriticReply::Final("junk one".into()),
        hs_loop::critic::CriticReply::Final("junk two".into()),
    ]);
    let r = hs_loop::critic::refute(
        &ws,
        "task",
        "checks",
        &hs_loop::critic::RefuteConfig::default(),
        &mut m,
    );
    assert!(!r.passed, "two consecutive unparseable verdicts must still fail closed");
    assert!(r.reason.contains("unparseable"), "honest reason: {}", r.reason);
}
