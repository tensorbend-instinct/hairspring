//! RED (2026-09-15, hs-research-repro stream 70ec2565-c023-4b26-9204-eed762f1c72a):
//! a live-surface research mission ("find today's latest AI research from
//! arXiv + preprint sources") submitted CORRECT, independently re-verified
//! work and still FAILED. The mission prompt on the normal run path is the
//! goal verbatim (TUI_MISSION_DEFAULT_TEMPLATE), so the .hs/checks contract
//! is discoverable only through the answer.submit tool schema - and the
//! contract forces the declaration to happen in a step BEFORE the
//! submission. The model learned it only from the post-submit "no checks
//! declared" feedback, wrote a green check on its final step, and the step
//! cap killed the mission (steps_exhausted, passed=false) one resubmit
//! short of the checker. THE LAW: the submission path must accept the
//! declaration ATOMICALLY - answer.submit's optional `checks` argument is
//! persisted to .hs/checks BEFORE the checker runs; the checker still
//! re-runs exactly those commands against the live machine (no bypass).

use hs_loop::repl::load_session;

static SEQMODEL_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

const TERMEXEC: &str = "/mnt/instinct-nvme/hairspring/target/debug/hs-plugin-termexec";
const ANSWERSUBMIT: &str = "/mnt/instinct-nvme/hairspring/target/debug/hs-plugin-answersubmit";
const CRITIC: &str = "/mnt/instinct-nvme/hairspring/target/debug/hs-plugin-critic";
const SCRIPTED: &str = "/mnt/instinct-nvme/hairspring/target/debug/hs-plugin-scripted";

fn write_rig(dir: &std::path::Path) {
    let toml = format!(
        r#"
[[tools]]
name = "term.exec"
command = ["{TERMEXEC}"]
subjects = ["*"]

[[tools]]
name = "answer.submit"
command = ["{ANSWERSUBMIT}"]
subjects = ["*"]

[[tools]]
name = "checker.run"
command = ["{CRITIC}"]
subjects = ["*"]

[[models]]
name = "scripted"
command = ["{SCRIPTED}"]
default = true
subjects = ["*"]
"#
    );
    std::fs::write(dir.join("hairspring.toml"), &toml).unwrap();
}

fn prep_env(script: &std::path::Path) {
    // FIXME: Audit that the environment access only happens in single-threaded code.
    unsafe {
        std::env::remove_var("HS_ANSWER_RAW");
        std::env::remove_var("HS_SELFCHECK_DIRECT");
        std::env::set_var("HS_SCRIPTED_PROMPT_AWARE", "1");
        std::env::set_var("HS_SEQMODEL_SCRIPT", script);
        std::env::set_var("HS_CRITIC_SCRIPT", "tool:grep -q hello hello.txt|clean");
    }
}

#[test]
fn inline_checks_persisted_before_checker_and_mission_goes_green() {
    let _g = SEQMODEL_LOCK.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let dir = std::env::temp_dir().join("repl-inline-checks-green");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    write_rig(&dir);
    let work = dir.join("run/work");
    let answer = work.join("write-hello-txt-containing-hello/answer.txt");
    let script = dir.join("script.jsonl");
    let line1 = r#"{"tool":"term.exec","args":{"command":"printf 'hello\\n' > hello.txt"}}"#;
    let line2 = format!(
        r#"{{"tool":"answer.submit","args":{{"path":"{}","summary":"wrote hello.txt containing hello; verified by the declared grep check","checks":"grep -q hello hello.txt"}}}}"#,
        answer.display()
    );
    std::fs::write(&script, format!("{line1}\n{line2}\n")).unwrap();
    prep_env(&script);
    let mut s = load_session(
        &dir.join("hairspring.toml"),
        &dir.join("run"),
        false,
        Some(12),
        None,
        None,
    )
    .unwrap();
    let r = s.run_goal("write hello txt containing hello").unwrap();
    assert!(
        r.passed,
        "a submission that declares its checks atomically must go green: {r:?}"
    );
    assert_eq!(
        std::fs::read_to_string(work.join(".hs/checks")).unwrap(),
        "grep -q hello hello.txt\n",
        "the inline declaration is persisted byte-exact before the checker runs"
    );
    assert!(
        std::fs::read_to_string(&answer)
            .unwrap()
            .contains("verified by the declared grep check"),
        "the submission itself still lands"
    );
    // FIXME: Audit that the environment access only happens in single-threaded code.
    unsafe { std::env::remove_var("HS_CRITIC_SCRIPT") };
}

#[test]
fn submit_without_checks_still_refused() {
    let _g = SEQMODEL_LOCK.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let dir = std::env::temp_dir().join("repl-inline-checks-absent");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    write_rig(&dir);
    let work = dir.join("run/work");
    let answer = work.join("write-hello-txt-containing-hello/answer.txt");
    let script = dir.join("script.jsonl");
    let line1 = r#"{"tool":"term.exec","args":{"command":"printf 'hello\\n' > hello.txt"}}"#;
    let line2 = format!(
        r#"{{"tool":"answer.submit","args":{{"path":"{}","summary":"wrote hello.txt containing hello"}}}}"#,
        answer.display()
    );
    std::fs::write(&script, format!("{line1}\n{line2}\n")).unwrap();
    prep_env(&script);
    let mut s = load_session(
        &dir.join("hairspring.toml"),
        &dir.join("run"),
        false,
        Some(6),
        None,
        None,
    )
    .unwrap();
    let r = s.run_goal("write hello txt containing hello").unwrap();
    assert!(
        !r.passed,
        "no declared checks anywhere must stay red (existing behavior): {r:?}"
    );
    assert!(
        !work.join(".hs/checks").exists(),
        "no checks file materializes out of thin air"
    );
    let verdict = hs_loop::selfcheck::check(&work);
    assert!(
        verdict["error"]
            .as_str()
            .unwrap()
            .contains("no checks declared"),
        "the checker still refuses an undeclared submission: {verdict}"
    );
    // FIXME: Audit that the environment access only happens in single-threaded code.
    unsafe { std::env::remove_var("HS_CRITIC_SCRIPT") };
}

#[test]
fn inline_checks_are_really_executed_no_bypass() {
    let _g = SEQMODEL_LOCK.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let dir = std::env::temp_dir().join("repl-inline-checks-failing");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    write_rig(&dir);
    let work = dir.join("run/work");
    let answer = work.join("write-hello-txt-containing-hello/answer.txt");
    let script = dir.join("script.jsonl");
    let line1 = r#"{"tool":"term.exec","args":{"command":"printf 'hello\\n' > hello.txt"}}"#;
    let line2 = format!(
        r#"{{"tool":"answer.submit","args":{{"path":"{}","summary":"claims done","checks":"false"}}}}"#,
        answer.display()
    );
    std::fs::write(&script, format!("{line1}\n{line2}\n")).unwrap();
    prep_env(&script);
    let mut s = load_session(
        &dir.join("hairspring.toml"),
        &dir.join("run"),
        false,
        Some(6),
        None,
        None,
    )
    .unwrap();
    let r = s.run_goal("write hello txt containing hello").unwrap();
    assert!(
        !r.passed,
        "a failing inline check must keep the mission red - declared checks are re-run, never trusted: {r:?}"
    );
    assert_eq!(
        std::fs::read_to_string(work.join(".hs/checks")).unwrap(),
        "false\n"
    );
    let verdict = hs_loop::selfcheck::check(&work);
    assert!(
        verdict["error"].as_str().unwrap().contains("`false`"),
        "the checker names the failing inline-declared command: {verdict}"
    );
    // FIXME: Audit that the environment access only happens in single-threaded code.
    unsafe { std::env::remove_var("HS_CRITIC_SCRIPT") };
}

#[test]
fn raw_schema_advertises_optional_checks() {
    let _g = SEQMODEL_LOCK.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    // FIXME: Audit that the environment access only happens in single-threaded code.
    unsafe { std::env::set_var("HS_ANSWER_RAW", "1") };
    let t = hs_loop::toolschema::schema_for("answer.submit", "apply").expect("answer.submit schema");
    unsafe { std::env::remove_var("HS_ANSWER_RAW") };
    let params = &t["function"]["parameters"];
    let required: Vec<&str> = params["required"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap())
        .collect();
    assert_eq!(required, vec!["path", "summary"], "required unchanged");
    assert!(
        params["properties"]["checks"]["type"].as_str() == Some("string"),
        "optional checks field advertised: {params}"
    );
    let desc = t["function"]["description"].as_str().unwrap();
    assert!(
        desc.contains("checks"),
        "the description teaches the atomic declaration channel: {desc}"
    );
}

#[test]
fn declare_checks_persists_and_refuses_empty() {
    let dir = tempfile::tempdir().unwrap();
    let n = hs_loop::selfcheck::declare_checks(dir.path(), "true\n# comment\nfalse\n").unwrap();
    assert_eq!(n, 2, "comments and blanks are not commands");
    assert_eq!(
        std::fs::read_to_string(dir.path().join(".hs/checks")).unwrap(),
        "true\n# comment\nfalse\n",
        "content lands byte-exact (trailing newline normalized)"
    );
    assert!(
        hs_loop::selfcheck::declare_checks(dir.path(), "  \n\n# only a comment\n").is_err(),
        "an empty declaration is refused, not silently dropped"
    );
}

#[test]
fn ten_step_research_mission_shape_goes_green_within_the_original_cap() {
    // Zero-cost scripted reproduction of hs-research-repro stream 70ec2565
    // (2026-09-15): a research-shaped mission that spent its steps gathering
    // evidence, then submitted at the step cap. The original run died at
    // steps_exhausted with passed=false one resubmit short, because the
    // checks declaration had to happen a step BEFORE the submission. With
    // atomic declaration the IDENTICAL step budget (10) now ends green.
    let _g = SEQMODEL_LOCK.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let dir = std::env::temp_dir().join("repl-inline-checks-tenstep");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    write_rig(&dir);
    let work = dir.join("run/work");
    let answer = work.join("find-todays-latest-ai-research/answer.txt");
    let script = dir.join("script.jsonl");
    let mut lines: Vec<String> = Vec::new();
    // Steps 1-8: research-shaped evidence gathering, mirroring the original
    // mission's 8 evidence fetches before its first submission attempt.
    for i in 1..=8 {
        lines.push(format!(
            r#"{{"tool":"term.exec","args":{{"command":"mkdir -p evidence && printf 'finding {i}\\n' > evidence/finding-{i}.txt"}}}}"#
        ));
    }
    // Step 9: submit WITH the atomic declaration (the fix). The original
    // mission submitted here with no checks and got "no checks declared".
    lines.push(format!(
        r#"{{"tool":"answer.submit","args":{{"path":"{}","summary":"today's latest AI research: 8 findings gathered and independently verified","checks":"test $(ls evidence | wc -l) -eq 8"}}}}"#,
        answer.display()
    ));
    std::fs::write(&script, lines.join("\n") + "\n").unwrap();
    prep_env(&script);
    // The canned critic still EXECUTES the declared check command itself
    // (tool: prefix) - the declaration is what is under test, not the
    // verdict. (Left unscripted, the critic refuted the placeholder
    // evidence on substance: the fail-closed path works.)
    unsafe {
        std::env::set_var(
            "HS_CRITIC_SCRIPT",
            "tool:test $(ls evidence | wc -l) -eq 8|clean",
        )
    };
    let mut s = load_session(
        &dir.join("hairspring.toml"),
        &dir.join("run"),
        false,
        Some(10),
        None,
        None,
    )
    .unwrap();
    let r = s.run_goal("find today's latest ai research").unwrap();
    assert!(
        r.passed,
        "the 10-step budget that killed the original mission now suffices: {r:?}"
    );
    assert!(
        r.steps <= 10,
        "green within the original 10-step cap, got {} steps",
        r.steps
    );
    assert_eq!(
        std::fs::read_to_string(work.join(".hs/checks")).unwrap(),
        "test $(ls evidence | wc -l) -eq 8\n",
        "the inline declaration persisted byte-exact"
    );
}

