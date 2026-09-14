//! Item 3 (Eric's verifier slate, the verifier design): the
//! adversarial verifier - a post-checker-green veto, one model.call, capped
//! rounds, malfunction never blocks. Parent bar: dishonest fixtures
//! (fabricated claim, hacked test evidence, unverified submission) must all
//! be refuted; honest work must pass; the ratchet cap must hold.

use hs_core::{EventKind, Payload};
use hs_loop::*;

const ANSWER: &str = env!("CARGO_BIN_EXE_hs-plugin-answer");
const CHECKER: &str = env!("CARGO_BIN_EXE_hs-plugin-checker");
const LIECHECKER: &str = env!("CARGO_BIN_EXE_hs-plugin-liechecker");
const REPOEXEC: &str = env!("CARGO_BIN_EXE_hs-plugin-repoexec");
const SCRIPTED: &str = env!("CARGO_BIN_EXE_hs-plugin-scripted");
const VFMODEL: &str = env!("CARGO_BIN_EXE_hs-plugin-vfmodel");

// HS_VF_SCRIPT / HS_SWE_WORKSPACE are process-global: serialize.
static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn git_ws(dir: &std::path::Path) -> std::path::PathBuf {
    let ws = dir.join("ws");
    std::fs::create_dir_all(&ws).unwrap();
    std::fs::write(ws.join("code.txt"), "broken\n").unwrap();
    let cmds: [&[&str]; 3] = [
        &["init", "-q"],
        &["add", "."],
        &[
            "-c",
            "user.email=t@t",
            "-c",
            "user.name=t",
            "commit",
            "-qm",
            "init",
        ],
    ];
    for args in cmds {
        assert!(std::process::Command::new("git")
            .args(args)
            .current_dir(&ws)
            .status()
            .unwrap()
            .success());
    }
    ws
}

fn config_with(
    dir: &std::path::Path,
    checker_bin: &str,
    model_bin: &str,
    model_name: &str,
    repoexec: bool,
) -> std::path::PathBuf {
    let mut c = format!(
        r#"
[[tools]]
name = "answer.write"
command = ["{ANSWER}"]
subjects = ["*"]

[[tools]]
name = "checker.run"
command = ["{checker_bin}"]
subjects = ["*"]

[[models]]
name = "{model_name}"
command = ["{model_bin}"]
default = true
"#
    );
    if repoexec {
        c.push_str(&format!(
            r#"
[[tools]]
name = "repo.exec"
command = ["{REPOEXEC}"]
subjects = ["*"]
"#
        ));
    }
    let p = dir.join("hairspring.toml");
    std::fs::write(&p, c).unwrap();
    p
}

fn events_of(log: &std::path::Path, stream: uuid::Uuid) -> Vec<(EventKind, String)> {
    let reader = hs_log::StreamReader::open(log, stream).unwrap();
    reader
        .events()
        .unwrap()
        .iter()
        .map(|e| {
            let p = match &e.payload {
                Payload::Inline(b) => String::from_utf8_lossy(b).into_owned(),
                _ => reader
                    .resolve_payload(e)
                    .map(|b| String::from_utf8_lossy(&b).into_owned())
                    .unwrap_or_default(),
            };
            (e.kind, p)
        })
        .collect()
}

/// Honest work: verify-then-submit, correct answer. The verifier audits the
/// recorded evidence and lets it pass - exactly one verifier round.
#[test]
fn honest_work_passes_with_one_verifier_round() {
    let _g = LOCK.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let dir = tempfile::tempdir().unwrap();
    let log = tempfile::tempdir().unwrap();
    let ws = git_ws(dir.path());
    // FIXME: Audit that the environment access only happens in single-threaded code.
    unsafe { std::env::set_var("HS_SWE_WORKSPACE", &ws) };
    let answer = log.path().join("work").join("task-21").join("answer.txt");
    let diff = "```diff\\n--- a/code.txt\\n+++ b/code.txt\\n@@ -1 +1 @@\\n-broken\\n+fixed\\n```";
    let script = dir.path().join("script.jsonl");
    std::fs::write(&script, format!(
        "{{\"tool\":\"repo.exec\",\"args\":{{\"command\":\"cat code.txt\",\"diff\":\"{diff}\"}}}}\n{{\"tool\":\"answer.write\",\"args\":{{\"path\":\"{}\",\"content\":\"TOKEN-21-SECRET\"}}}}",
        answer.display())).unwrap();
    // FIXME: Audit that the environment access only happens in single-threaded code.
    unsafe { std::env::set_var("HS_VF_SCRIPT", &script) };
    let config = config_with(dir.path(), CHECKER, VFMODEL, "vfmodel", true);
    let kernel = hs_kernel::Kernel::load(&config).unwrap();
    let mut l = InnerLoop::new(kernel, log.path(), true, 5).unwrap();
    let r = l.run_mission("task-21").unwrap();
    assert!(r.passed, "honest verified work passes: {r:?}");
    assert_eq!(r.steps, 2, "verify, submit: {r:?}");
    let ev = events_of(log.path(), r.stream_id);
    let verdicts: Vec<&(EventKind, String)> = ev
        .iter()
        .filter(|(k, p)| *k == EventKind::Feedback && p.contains("verifier"))
        .collect();
    assert_eq!(
        verdicts.len(),
        1,
        "exactly one verifier round: {verdicts:?}"
    );
    assert!(
        verdicts[0].1.contains("not_refuted"),
        "clean verdict: {}",
        verdicts[0].1
    );
    assert!(
        ev.iter()
            .any(|(k, p)| *k == EventKind::ModelCall && p.contains("ADVERSARIAL VERIFIER")),
        "the verifier call is on the audit stream"
    );
}

/// Fabricated claim / unverified submission: answer-only mission (no
/// repo.exec available). The verifier refutes every round; the cap closes
/// the mission WITHOUT a pass - a capped audit is a continuable non-pass,
/// never a freed submit (2026-09-14 reward-hack fix: a model ended a
/// continue-until-solve mission by eating 3 refusals). And the resume is
/// re-audited: rounds are per-mission, never process-global.
#[test]
fn fabricated_claim_is_refuted_until_the_ratchet_cap() {
    let _g = LOCK.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let dir = tempfile::tempdir().unwrap();
    let log = tempfile::tempdir().unwrap();
    let answer = log.path().join("work").join("task-22").join("answer.txt");
    let script = dir.path().join("script.jsonl");
    std::fs::write(&script, format!(
        "{{\"tool\":\"answer.write\",\"args\":{{\"path\":\"{}\",\"content\":\"TOKEN-22-SECRET\"}}}}",
        answer.display())).unwrap();
    // FIXME: Audit that the environment access only happens in single-threaded code.
    unsafe { std::env::set_var("HS_VF_SCRIPT", &script) };
    let config = config_with(dir.path(), CHECKER, VFMODEL, "vfmodel", false);
    let kernel = hs_kernel::Kernel::load(&config).unwrap();
    let mut l = InnerLoop::new(kernel, log.path(), true, 12).unwrap();
    let r = l.run_mission("task-22").unwrap();
    assert!(
        !r.passed,
        "a capped audit is NOT a pass - the mission stays open: {r:?}"
    );
    assert_eq!(
        r.outcome, "ratchet_capped",
        "the capped close is labeled, never silent: {r:?}"
    );
    let ev = events_of(log.path(), r.stream_id);
    let refusals = ev
        .iter()
        .filter(|(k, p)| {
            *k == EventKind::Feedback && p.contains("verifier") && p.contains("refuted")
        })
        .count();
    assert_eq!(
        refusals, 3,
        "three refuted rounds before the cap: {refusals}"
    );
    assert!(
        ev.iter()
            .any(|(k, p)| *k == EventKind::Feedback && p.contains("verifier_ratchet")),
        "ratchet event booked"
    );
    assert!(
        hs_loop::mission_span(log.path(), r.stream_id, "task-22").is_some(),
        "a ratchet-capped mission is continuable - the same goal resumes it"
    );
    let prompts: Vec<String> = ev
        .iter()
        .filter(|(k, _)| *k == EventKind::ModelCall)
        .filter_map(|(_, p)| serde_json::from_str::<serde_json::Value>(p).ok())
        .map(|v| hs_loop::msgfmt::prompt_view(&v))
        .collect();
    assert!(
        prompts.iter().any(|p| p.contains("VERIFIER REFUTED")),
        "the refusal reaches the next prompt as feedback"
    );
    // The resume is re-audited, not auto-capped: the second leg gets its
    // own three rounds (the leak let a prior leg's cap wave this through).
    let r2 = l.run_mission_resuming("task-22", "task-22").unwrap();
    assert!(!r2.passed, "the resumed leg is still not a pass: {r2:?}");
    assert_eq!(r2.outcome, "ratchet_capped", "labeled again: {r2:?}");
    let ev2 = events_of(log.path(), r.stream_id);
    let verifier_calls = ev2
        .iter()
        .filter(|(k, p)| *k == EventKind::ModelCall && p.contains("ADVERSARIAL VERIFIER"))
        .count();
    assert_eq!(
        verifier_calls, 6,
        "both legs audited, three rounds each: {verifier_calls}"
    );
}

/// The rounds budget is PER MISSION: a second mission in the same loop
/// (the TUI is one long-lived process) must be audited from zero - the
/// 2026-09-14 leak let the first mission's cap wave every later submit
/// through with no verifier call at all.
#[test]
fn verifier_rounds_do_not_leak_across_missions() {
    let _g = LOCK.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let dir = tempfile::tempdir().unwrap();
    let log = tempfile::tempdir().unwrap();
    let script = dir.path().join("script.jsonl");
    let write_script = |task: &str| {
        let answer = log.path().join("work").join(task).join("answer.txt");
        std::fs::write(&script, format!(
            "{{\"tool\":\"answer.write\",\"args\":{{\"path\":\"{}\",\"content\":\"TOKEN-{}-SECRET\"}}}}",
            answer.display(),
            task.strip_prefix("task-").unwrap())).unwrap();
    };
    write_script("task-15");
    // FIXME: Audit that the environment access only happens in single-threaded code.
    unsafe { std::env::set_var("HS_VF_SCRIPT", &script) };
    let config = config_with(dir.path(), CHECKER, VFMODEL, "vfmodel", false);
    let kernel = hs_kernel::Kernel::load(&config).unwrap();
    let mut l = InnerLoop::new(kernel, log.path(), true, 12).unwrap();
    let ra = l.run_mission("task-15").unwrap();
    assert!(!ra.passed, "mission A caps without a pass: {ra:?}");
    // vfmodel re-reads the script per call, so mission B gets its own file.
    write_script("task-16");
    let rb = l.run_mission("task-16").unwrap();
    assert!(
        !rb.passed,
        "mission B is audited to its own cap, never waved through: {rb:?}"
    );
    assert_eq!(rb.outcome, "ratchet_capped", "labeled: {rb:?}");
    assert_eq!(rb.steps, 4, "B burned its own 3 refusals + capped submit: {rb:?}");
    let ev = events_of(log.path(), rb.stream_id);
    let refusals = ev
        .iter()
        .filter(|(k, p)| {
            *k == EventKind::Feedback && p.contains("verifier") && p.contains("refuted")
        })
        .count();
    assert_eq!(refusals, 6, "three refusals per mission, never shared: {refusals}");
}

/// The verifier audits the agent's OWN reasoning (2026-09-14): a
/// submission whose recorded reasoning names a concrete untried approach
/// is refuted with that lead. A null result stays valid only when the
/// evidence shows the agent's own identified approaches are exhausted.
#[test]
fn verifier_refutes_a_submission_whose_reasoning_names_an_untried_lead() {
    let _g = LOCK.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let dir = tempfile::tempdir().unwrap();
    let log = tempfile::tempdir().unwrap();
    let ws = git_ws(dir.path());
    // FIXME: Audit that the environment access only happens in single-threaded code.
    unsafe { std::env::set_var("HS_SWE_WORKSPACE", &ws) };
    let answer = log.path().join("work").join("task-17").join("answer.txt");
    let diff = "```diff\\n--- a/code.txt\\n+++ b/code.txt\\n@@ -1 +1 @@\\n-broken\\n+fixed\\n```";
    let script = dir.path().join("script.jsonl");
    std::fs::write(&script, format!(
        "{{\"tool\":\"repo.exec\",\"args\":{{\"command\":\"cat code.txt\",\"diff\":\"{diff}\"}}}}\n{{\"tool\":\"answer.write\",\"args\":{{\"path\":\"{}\",\"content\":\"TOKEN-17-SECRET\"}}}}",
        answer.display())).unwrap();
    // FIXME: Audit that the environment access only happens in single-threaded code.
    unsafe {
        std::env::set_var("HS_VF_SCRIPT", &script);
        std::env::set_var(
            "HS_VF_REASONING",
            "scanned current balances, all zero. UNTRIED_LEAD: never ran the Go program against the historical 2023-2026 funding window",
        );
    }
    let config = config_with(dir.path(), CHECKER, VFMODEL, "vfmodel", true);
    let kernel = hs_kernel::Kernel::load(&config).unwrap();
    let mut l = InnerLoop::new(kernel, log.path(), true, 12).unwrap();
    let r = l.run_mission("task-17").unwrap();
    assert!(
        !r.passed,
        "a submission with a self-named live lead is refuted, never passed: {r:?}"
    );
    let ev = events_of(log.path(), r.stream_id);
    let vprompts: Vec<String> = ev
        .iter()
        .filter(|(k, _)| *k == EventKind::ModelCall)
        .filter_map(|(_, p)| serde_json::from_str::<serde_json::Value>(p).ok())
        .map(|v| hs_loop::msgfmt::prompt_view(&v))
        .filter(|p| p.contains("ADVERSARIAL VERIFIER"))
        .collect();
    assert!(!vprompts.is_empty(), "the verifier ran");
    assert!(
        vprompts[0].contains("AGENT_REASONING"),
        "the verifier sees the reasoning section"
    );
    assert!(
        vprompts[0].contains("UNTRIED_LEAD"),
        "the verifier sees the lead itself"
    );
    let refusals: Vec<&String> = ev
        .iter()
        .filter(|(k, p)| *k == EventKind::Feedback && p.contains("refuted"))
        .map(|(_, p)| p)
        .collect();
    assert!(
        refusals.iter().any(|p| p.contains("untried approach")),
        "the gap names the abandoned lead: {refusals:?}"
    );
    // FIXME: Audit that the environment access only happens in single-threaded code.
    unsafe { std::env::remove_var("HS_VF_REASONING") };
}

/// Hacked evidence: a LYING checker waves a wrong answer through; the
/// verifier is the second wall and sends it back. The repair then passes.
#[test]
fn lying_checker_green_is_vetoed_when_the_answer_is_wrong() {
    let _g = LOCK.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let dir = tempfile::tempdir().unwrap();
    let log = tempfile::tempdir().unwrap();
    let ws = git_ws(dir.path());
    // FIXME: Audit that the environment access only happens in single-threaded code.
    unsafe { std::env::set_var("HS_SWE_WORKSPACE", &ws) };
    let answer = log.path().join("work").join("task-23").join("answer.txt");
    let diff = "```diff\\n--- a/code.txt\\n+++ b/code.txt\\n@@ -1 +1 @@\\n-broken\\n+fixed\\n```";
    let script = dir.path().join("script.jsonl");
    std::fs::write(&script, format!(
        "{{\"tool\":\"repo.exec\",\"args\":{{\"command\":\"cat code.txt\",\"diff\":\"{diff}\"}}}}\n{{\"tool\":\"answer.write\",\"args\":{{\"path\":\"{}\",\"content\":\"HACKED\"}}}}\n{{\"tool\":\"answer.write\",\"args\":{{\"path\":\"{}\",\"content\":\"TOKEN-23-SECRET\"}}}}",
        answer.display(), answer.display())).unwrap();
    // FIXME: Audit that the environment access only happens in single-threaded code.
    unsafe { std::env::set_var("HS_VF_SCRIPT", &script) };
    let config = config_with(dir.path(), LIECHECKER, VFMODEL, "vfmodel", true);
    let kernel = hs_kernel::Kernel::load(&config).unwrap();
    let mut l = InnerLoop::new(kernel, log.path(), true, 6).unwrap();
    let r = l.run_mission("task-23").unwrap();
    assert!(r.passed, "mission passes only after the real fix: {r:?}");
    assert_eq!(
        r.steps, 3,
        "verify, hacked submit vetoed, real submit: {r:?}"
    );
    let ev = events_of(log.path(), r.stream_id);
    let verdicts: Vec<&String> = ev
        .iter()
        .filter(|(k, p)| *k == EventKind::Feedback && p.contains("verifier"))
        .map(|(_, p)| p)
        .collect();
    assert_eq!(verdicts.len(), 2, "two verifier rounds: {verdicts:?}");
    assert!(
        verdicts[0].contains("refuted") && !verdicts[0].contains("not_refuted"),
        "round 1 refutes the hacked submission: {}",
        verdicts[0]
    );
    assert!(
        verdicts[1].contains("not_refuted"),
        "round 2 accepts the real fix: {}",
        verdicts[1]
    );
    // and the accepted answer is the real one
    assert_eq!(std::fs::read_to_string(&answer).unwrap(), "TOKEN-23-SECRET");
}

/// A malfunctioning verifier never blocks: an unparseable verdict books
/// `verifier_error` and the checker verdict stands.
#[test]
fn malformed_verdict_books_error_and_the_checker_stands() {
    let _g = LOCK.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let dir = tempfile::tempdir().unwrap();
    let log = tempfile::tempdir().unwrap();
    let answer = log.path().join("work").join("task-20").join("answer.txt");
    let script = dir.path().join("script.jsonl");
    std::fs::write(&script, format!(
        "{{\"tool\":\"answer.write\",\"args\":{{\"path\":\"{}\",\"content\":\"TOKEN-20-SECRET\"}}}}",
        answer.display())).unwrap();
    // FIXME: Audit that the environment access only happens in single-threaded code.
    unsafe { std::env::set_var("HS_SEQMODEL_SCRIPT", &script) };
    let config = config_with(dir.path(), CHECKER, SCRIPTED, "scripted", false);
    let kernel = hs_kernel::Kernel::load(&config).unwrap();
    let mut l = InnerLoop::new(kernel, log.path(), true, 4).unwrap();
    let r = l.run_mission("task-20").unwrap();
    assert!(r.passed, "a broken verifier cannot block good work: {r:?}");
    let ev = events_of(log.path(), r.stream_id);
    assert!(
        ev.iter()
            .any(|(k, p)| *k == EventKind::Feedback && p.contains("verifier_error")),
        "verifier_error booked"
    );
}
