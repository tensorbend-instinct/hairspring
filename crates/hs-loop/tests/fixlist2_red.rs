//! RED: post-A9/A10 fix list (batch-1 gate, 2026-09-05).
//! 1. reasoning_content captured from provider responses (was: count only,
//!    the content itself dropped) - ModelCall observability.
//! 2. Goal evaluator environmental-red classification: an f2p run that
//!    fails because the exec sandbox lacks the tool (exit 127 /
//!    "command not found") is EnvLimited, never a plain red Fail (A7's
//!    22-step / $1.46 environmental veto class).
//! 3. Guardrail escalation: repeated same-class edit-path violations
//!    escalate (B8: 6 same-class fires, the model never took the steer) -
//!    a GuardrailEscalator counts per class and emits a strengthened
//!    steer from the second fire on.
//! 4. MCP bridge binary resolution: sibling of the running exe,
//!    canonicalized; a missing sibling is a loud error, never a silent
//!    pickup from another tree.
//! 5. Bypass steering quality: the refusal names the violation class and
//!    shows a concrete edit.apply example, not just a rule.

use hs_loop::*;

// ---- 1: reasoning_content ----

#[test]
fn parse_response_captures_reasoning_content() {
    let p = realmodel::glm();
    let v = serde_json::json!({
        "choices": [{"finish_reason": "tool_calls", "message": {
            "reasoning_content": "the ignore matcher needs an inverse flag",
            "tool_calls": [{"function": {"name": "repo__read", "arguments": "{\"path\":\"a.rs\"}"}}]
        }}],
        "usage": {"prompt_tokens": 10, "completion_tokens": 5}
    });
    let out = realmodel::parse_response(&p, &v).unwrap();
    assert_eq!(
        out.reasoning_content, "the ignore matcher needs an inverse flag",
        "reasoning_content must survive parsing"
    );
}

#[test]
fn parse_response_reasoning_content_absent_is_empty() {
    let p = realmodel::glm();
    let v = serde_json::json!({
        "choices": [{"finish_reason": "tool_calls", "message": {
            "tool_calls": [{"function": {"name": "repo__read", "arguments": "{}"}}]
        }}],
        "usage": {"prompt_tokens": 1, "completion_tokens": 1}
    });
    let out = realmodel::parse_response(&p, &v).unwrap();
    assert_eq!(out.reasoning_content, "");
}

// ---- 2: env-limited goal verdicts ----

#[test]
fn env_failure_classified_from_exec_result() {
    let r = serde_json::json!({"applied": true, "exit_code": 127,
        "stderr": "bash: pytest: command not found", "stdout": ""});
    let cls = goal::classify_env_failure(&r);
    assert!(
        cls.is_some(),
        "exit 127 + command not found is environmental"
    );
    assert!(cls.unwrap().contains("pytest"));
}

#[test]
fn real_test_failure_is_not_env_limited() {
    let r = serde_json::json!({"applied": true, "exit_code": 1,
        "stderr": "FAILED test/integration/x.py::test_y - AssertionError", "stdout": ""});
    assert!(goal::classify_env_failure(&r).is_none());
}

#[test]
fn goal_verdict_enum_covers_env_limited() {
    let v = goal::GoalVerdict::EnvLimited("pytest missing".into());
    match v {
        goal::GoalVerdict::Pass | goal::GoalVerdict::Fail => panic!("wrong variant"),
        goal::GoalVerdict::EnvLimited(r) => assert!(r.contains("pytest")),
    }
}

// ---- 3: guardrail escalation ----

#[test]
fn violation_classes() {
    assert_eq!(
        repexec::violation_class("git apply invocation"),
        "git_apply"
    );
    assert_eq!(
        repexec::violation_class("raw diff-file write (> x.diff)"),
        "diff_write"
    );
}

#[test]
fn escalator_silent_on_first_fire_then_escalates() {
    let mut esc = repexec::GuardrailEscalator::new();
    assert!(
        esc.record("git_apply").is_none(),
        "first fire: the gate's own steer speaks"
    );
    let n2 = esc
        .record("git_apply")
        .expect("second same-class fire escalates");
    assert!(n2.contains("git_apply") && n2.contains('2'), "{n2}");
    assert!(n2.contains("edit.apply"), "{n2}");
    let n3 = esc.record("git_apply").expect("third fire escalates again");
    assert!(n3.contains('3'), "{n3}");
    assert!(
        esc.record("diff_write").is_none(),
        "a different class starts its own count"
    );
}

// ---- 4: mcp bridge binary resolution ----

#[test]
fn resolve_plugin_bin_sibling_or_loud_error() {
    let dir = tempfile::tempdir().unwrap();
    let exe = dir.path().join("hs-swe-run");
    std::fs::write(&exe, b"").unwrap();
    let err = mcpbridge::resolve_plugin_bin(&exe, "hs-plugin-mcpcall").unwrap_err();
    assert!(err.contains("hs-plugin-mcpcall"), "{err}");
    std::fs::write(dir.path().join("hs-plugin-mcpcall"), b"").unwrap();
    let got = mcpbridge::resolve_plugin_bin(&exe, "hs-plugin-mcpcall").unwrap();
    assert_eq!(
        got.file_name().unwrap().to_str().unwrap(),
        "hs-plugin-mcpcall"
    );
}

// ---- 5: steering text quality ----

#[test]
fn edit_gate_steer_names_class_and_shows_example() {
    let gate = repexec::run_sandboxed(
        std::path::Path::new("/nonexistent-ws"),
        std::path::Path::new("/nonexistent-answer"),
        "git apply /tmp/x.diff",
        5,
    );
    let err = gate["error"].as_str().unwrap_or("");
    assert!(err.contains("edit.apply"), "{err}");
    assert!(err.contains("class: git_apply"), "{err}");
    assert!(err.contains("search") && err.contains("replace"), "{err}");
}

#[test]
fn gate_output_yields_class_for_escalation() {
    let gate = repexec::run_sandboxed(
        std::path::Path::new("/nonexistent-ws"),
        std::path::Path::new("/nonexistent-answer"),
        "git apply /tmp/x.diff",
        5,
    );
    let out = gate.to_string();
    assert_eq!(
        repexec::extract_gate_class(&out).as_deref(),
        Some("git_apply")
    );
}
