//! RED: repo.exec edit-path guardrail (post-B7, 2026-09-05).
//! B7's model bypassed edit.apply: it hand-wrote raw diffs (/tmp/p26.diff,
//! /tmp/candidate.diff) and git-applied them through the scratch shell. The
//! harvested checker patch came back "corrupt patch at line 172" - the same
//! model-written-diff failure class the splice migration killed, alive
//! through the exec backdoor. New contract: repo.exec is build/test ONLY.
//! - `git apply` invocations are rejected (the whole class: --check too)
//! - raw diff-file WRITES are rejected (>, >>, tee, cp/mv/install targets
//!   ending .diff/.patch)
//!
//! Both return applied:false + a steering error naming edit.apply, and the
//! command must NOT execute. Reads of .diff files, git diff output to
//! stdout, and every other shell command stay allowed.
//! Levels:
//! - unit: edit_path_violation over the forbidden/allowed shapes
//! - sandbox: all three run_* entry points enforce the gate, command never
//!   runs, result steers to edit.apply; an allowed command still executes
//! - prompt: mission template + repo.exec schema description carry the
//!   "edits only via edit.apply" steering
//! - mission: scripted model replays B7's verbatim payload, gets steered,
//!   keeps working, mission still passes

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
fn gate_flags_git_apply_invocations() {
    for cmd in [
        "git apply /tmp/p26.diff",
        "git apply --check /tmp/candidate.diff",
        "git -C /ws apply p.diff",
        "git -c core.fileMode=false apply --stat p.diff",
        "git --git-dir=/ws/.git apply p.diff",
        // B7's verbatim shape: chained, absolute paths
        "cd /ws && md5sum /tmp/candidate.diff && git apply --check /tmp/candidate.diff && git apply /tmp/candidate.diff",
    ] {
        let v = hs_loop::repexec::edit_path_violation(cmd);
        assert!(v.is_some(), "must forbid: {cmd}");
        assert!(v.unwrap().contains("git apply"), "reason names the class: {cmd}");
    }
}

#[test]
fn gate_flags_raw_diff_file_writes() {
    for cmd in [
        "git diff > /tmp/p26.diff",
        "git diff >> p.patch",
        "cat > fix.diff <<'EOF'",
        "python3 gen.py > out.patch",
        "git diff 2>/dev/null > candidate.diff",
        "git diff | tee /tmp/p.diff",
        "git diff | tee -a p.patch",
        "cp /tmp/a.txt /tmp/b.diff",
        "mv x.txt y.patch",
        "install -m 644 a.txt b.diff",
    ] {
        let v = hs_loop::repexec::edit_path_violation(cmd);
        assert!(v.is_some(), "must forbid: {cmd}");
        assert!(
            v.unwrap().contains("diff-file"),
            "reason names the class: {cmd}"
        );
    }
}

#[test]
fn gate_allows_normal_shell_and_reads() {
    for cmd in [
        "git log --oneline | head -1",
        "git diff --stat",
        "git diff",
        "git show HEAD --stat",
        "git branch apply-fixes",
        "grep -rn apply src/",
        "cat existing.diff",
        "less fixture.patch",
        "md5sum /tmp/candidate.diff",
        "ls -la /tmp/p26.diff",
        "cargo test",
        "python3 -m pytest test/ -q",
        "git checkout -q test",
        "git status",
    ] {
        let v = hs_loop::repexec::edit_path_violation(cmd);
        assert!(v.is_none(), "must allow: {cmd} (got {v:?})");
    }
}

#[test]
fn sandbox_rejects_b7s_verbatim_git_apply_payload() {
    std::env::remove_var("HS_SWE_ANSWER");
    let ws = mk_ws();
    let r = hs_loop::repexec::run_sandboxed_no_patch(
        ws.path(),
        "cd /ws && git apply --check /tmp/candidate.diff && git apply /tmp/candidate.diff && git diff --stat",
        30,
    );
    assert_eq!(r["applied"], false, "nothing applied: {r}");
    assert_eq!(r["exit_code"], -1, "command must NOT execute: {r}");
    assert_eq!(r["stdout"], "", "no output - command never ran: {r}");
    let err = r["error"].as_str().expect("steering error present");
    assert!(err.contains("edit.apply"), "steers to edit.apply: {err}");
    assert!(err.contains("git apply"), "names the violation: {err}");
}

#[test]
fn sandbox_gates_diff_writes_before_any_work() {
    std::env::remove_var("HS_SWE_ANSWER");
    let ws = mk_ws();
    let r = hs_loop::repexec::run_sandboxed_no_patch(
        ws.path(),
        "git diff > /tmp/p26.diff && echo WROTE",
        30,
    );
    assert_eq!(r["applied"], false, "{r}");
    assert_eq!(r["exit_code"], -1, "command must NOT execute: {r}");
    let err = r["error"].as_str().expect("steering error present");
    assert!(err.contains("edit.apply"), "steers to edit.apply: {err}");
    assert!(err.contains("diff-file"), "names the violation: {err}");
}

#[test]
fn diff_mode_and_answer_mode_are_gated_too() {
    let ws = mk_ws();
    let r = hs_loop::repexec::run_sandboxed_with_diff(
        ws.path(),
        "--- a/code.txt\n+++ b/code.txt\n@@ -1 +1 @@\n-broken\n+fixed\n",
        "git apply --check /tmp/other.diff",
        30,
    );
    assert_eq!(r["applied"], false, "{r}");
    assert_eq!(r["exit_code"], -1, "command must NOT execute: {r}");
    assert!(
        r["error"].as_str().unwrap_or("").contains("edit.apply"),
        "{r}"
    );
}

#[test]
fn allowed_command_still_executes_end_to_end() {
    std::env::remove_var("HS_SWE_ANSWER");
    let ws = mk_ws();
    let r = hs_loop::repexec::run_sandboxed_no_patch(
        ws.path(),
        "git log --oneline | head -1; git diff --stat; grep -c broken code.txt",
        30,
    );
    assert_eq!(r["exit_code"], 0, "allowed shell runs: {r}");
    assert!(r["stdout"].as_str().unwrap().contains("init"), "{r}");
    assert!(
        r.get("error").is_none(),
        "no steering error on allowed commands: {r}"
    );
}

#[test]
fn prompt_steers_edits_only_via_edit_apply() {
    // 2026-09-06: the edit path is now edit.patch (Codex apply_patch
    // grammar); the guardrail property is unchanged: exactly ONE named edit
    // path, git apply forbidden.
    // 2026-09-06 bake-off: the template carries {edit_tool}/{edit_policy}
    // placeholders; each arm must render its own tool name and never the
    // other arm's.
    assert!(
        hs_loop::sweprompt::SWE_MISSION_TEMPLATE.contains("{edit_tool}"),
        "mission template must carry the edit-tool placeholder"
    );
    let args = hs_loop::sweprompt::PromptArgs {
        ws: "/tmp/ws".into(),
        problem_statement: "p".into(),
        fail_to_pass: vec!["t".into()],
        repo_layout: "src/main.rs\n".into(),
        nudge: String::new(),
        answer_path: "/tmp/a".into(),
        orientation: String::new(),
        mcp_tools: String::new(),
    };
    std::env::remove_var("HS_SWE_EDIT_PATH");
    let p_default = hs_loop::sweprompt::build_mission_prompt(None, &args);
    assert!(
        p_default.contains("edit.patch"),
        "default arm renders edit.patch"
    );
    assert!(
        !p_default.contains("edit.anchor"),
        "default arm never names edit.anchor"
    );
    std::env::set_var("HS_SWE_EDIT_PATH", "anchor");
    let p_anchor = hs_loop::sweprompt::build_mission_prompt(None, &args);
    std::env::remove_var("HS_SWE_EDIT_PATH");
    assert!(
        p_anchor.contains("edit.anchor"),
        "anchor arm renders edit.anchor"
    );
    assert!(
        !p_anchor.contains("edit.patch"),
        "anchor arm never names edit.patch: {}",
        p_anchor
            .lines()
            .filter(|l| l.contains("edit.patch"))
            .collect::<Vec<_>>()
            .join(" || ")
    );
    let tools = hs_loop::toolschema::builtin_tools();
    let exec = tools
        .iter()
        .find(|t| t["function"]["name"] == "repo.exec")
        .expect("repo.exec schema");
    let desc = exec["function"]["description"].as_str().unwrap();
    assert!(
        desc.contains("edit.patch"),
        "repo.exec description steers to edit.patch: {desc}"
    );
    assert!(
        desc.contains("git apply"),
        "repo.exec description names the forbidden class: {desc}"
    );
}

/// Mission level: B7's failure shape, replayed through the real
/// loop/kernel/plugin path. The scripted model first issues B7's verbatim
/// git-apply payload (must come back forbidden, steering to edit.apply),
/// then a normal shell command (must run), then writes the answer. The
/// mission must still pass - the guardrail steers, it does not doom.
#[test]
fn mission_git_apply_payload_is_steered_not_executed() {
    std::env::remove_var("HS_SWE_ANSWER");
    let dir = tempfile::tempdir().unwrap();
    let log = tempfile::tempdir().unwrap();
    let ws = mk_ws();
    std::env::set_var("HS_SWE_WORKSPACE", ws.path());
    let answer = log.path().join("work").join("task-0").join("answer.txt");
    let script = dir.path().join("script.jsonl");
    std::fs::write(
        &script,
        format!(
            "{{\"tool\":\"repo.exec\",\"args\":{{\"command\":\"cd /ws && git apply --check /tmp/candidate.diff && git apply /tmp/candidate.diff\"}}}}\n{{\"tool\":\"repo.exec\",\"args\":{{\"command\":\"git log --oneline | head -1\"}}}}\n{{\"tool\":\"answer.write\",\"args\":{{\"path\":\"{}\",\"content\":\"TOKEN-0-SECRET\"}}}}",
            answer.display()
        ),
    )
    .unwrap();
    std::env::set_var("HS_SEQMODEL_SCRIPT", &script);
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
    let r = l.run_mission("task-0").unwrap();
    assert!(
        r.passed,
        "guardrail steers without dooming the mission: {r:?}"
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
    let mut exec_results = events.iter().filter_map(|e| {
        let p = String::from_utf8_lossy(&reader.resolve_payload(e).unwrap()).to_string();
        p.contains("\"plugin\":\"repo.exec\"").then_some(p)
    });
    let forbidden = exec_results
        .next()
        .expect("first repo.exec result on the stream");
    assert!(
        forbidden.contains("\"applied\":false"),
        "git-apply payload not applied: {forbidden}"
    );
    assert!(
        forbidden.contains("\"exit_code\":-1"),
        "git-apply payload never executed: {forbidden}"
    );
    assert!(
        forbidden.contains("edit.apply"),
        "result steers to edit.apply: {forbidden}"
    );
    let allowed = exec_results
        .next()
        .expect("second repo.exec result on the stream");
    assert!(
        allowed.contains("\"exit_code\":0"),
        "normal shell still runs after a steered call: {allowed}"
    );

    std::env::remove_var("HS_SWE_WORKSPACE");
    std::env::remove_var("HS_SEQMODEL_SCRIPT");
}
