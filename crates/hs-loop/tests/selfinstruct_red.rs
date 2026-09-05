//! RED contract tests: agent self-instruction (spec gate 8: "the agent
//! rewrites a prompt and a tool"; bootstrap rule: nothing self-modifies
//! until the assay maturity gate - so proposals are recorded, promotion is
//! gated out-of-mission). The SWE mission prompt is sourced from the policy
//! layer; a model proposal is versioned + auditable and NEVER changes the
//! running mission.

use hs_loop::sweprompt::*;

fn args() -> PromptArgs {
    PromptArgs {
        ws: "/tmp/ws".into(),
        problem_statement: "bug: stack mishandled".into(),
        fail_to_pass: vec!["pytest test_x -x".into()],
        repo_layout: "src/main.rs\n".into(),
        nudge: String::new(),
        answer_path: "/tmp/answer.txt".into(),
        orientation: String::new(),
    }
}

#[test]
fn default_template_when_no_policy_overlay() {
    let p = build_mission_prompt(None, &args());
    assert!(p.contains("You are fixing a real bug"), "builtin template: {p}");
    assert!(p.contains("bug: stack mishandled"));
    assert!(p.contains("repo.exec"), "tool surface listed");
    // allowlist scaffold was deleted per Eric's ruling - no list in the prompt
    assert!(!p.contains("Allowed:"), "no allowlist mention: {p}");
    assert!(!p.contains("{ws}"), "all placeholders substituted: {p}");
}

#[test]
fn policy_overlay_overrides_mission_prompt() {
    let overlay = r#"
[prompts]
swe-mission = "CUSTOM POLICY PROMPT for {ws} about {problem_statement}"
"#;
    let f = tempfile::NamedTempFile::new().unwrap();
    std::fs::write(f.path(), overlay).unwrap();
    let pol = load_policy_overlay(f.path()).unwrap();
    let p = build_mission_prompt(Some(&pol), &args());
    assert!(p.starts_with("CUSTOM POLICY PROMPT for /tmp/ws"), "overlay used: {p}");
    assert!(p.contains("bug: stack mishandled"));
    assert!(!p.contains("You are fixing a real bug"), "builtin replaced");
}

#[test]
fn overlay_missing_mission_key_falls_back_to_builtin() {
    let overlay = "[prompts]\nother = \"x\"\n";
    let f = tempfile::NamedTempFile::new().unwrap();
    std::fs::write(f.path(), overlay).unwrap();
    let pol = load_policy_overlay(f.path()).unwrap();
    let p = build_mission_prompt(Some(&pol), &args());
    assert!(p.contains("You are fixing a real bug"));
}

#[test]
fn malformed_overlay_is_an_error_not_a_silent_default() {
    let f = tempfile::NamedTempFile::new().unwrap();
    std::fs::write(f.path(), "[prompts\nbroken").unwrap();
    assert!(load_policy_overlay(f.path()).is_err());
}

#[test]
fn proposal_is_recorded_versioned_and_does_not_mutate_running_prompt() {
    let dir = tempfile::tempdir().unwrap();
    let before = build_mission_prompt(None, &args());
    let rec1 = propose_prompt(dir.path(), "swe-mission", "always repo.exec first").unwrap();
    let rec2 = propose_prompt(dir.path(), "swe-mission", "v2 text").unwrap();
    // versioned lineage: parent hashes chain
    assert_eq!(rec1.version, 1);
    assert_eq!(rec2.version, 2);
    assert_eq!(rec2.parent_hash, rec1.hash);
    assert_ne!(rec1.hash, rec2.hash);
    // auditable on disk
    let log = std::fs::read_to_string(dir.path().join("policy_proposals.jsonl")).unwrap();
    assert_eq!(log.lines().count(), 2);
    assert!(log.contains("always repo.exec first"));
    assert!(log.contains("\"status\":\"proposed\""), "gated, not applied: {log}");
    // the running mission's prompt is untouched (quarantine)
    let after = build_mission_prompt(None, &args());
    assert_eq!(before, after);
}
