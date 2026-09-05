//! Item 3 (Eric's verifier slate, design docs/verifier-design.md): the
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
    let cmds: [&[&str]; 3] = [&["init", "-q"], &["add", "."], &["-c", "user.email=t@t", "-c", "user.name=t", "commit", "-qm", "init"]];
    for args in cmds {
        assert!(std::process::Command::new("git").args(args).current_dir(&ws).status().unwrap().success());
    }
    ws
}

fn config_with(dir: &std::path::Path, checker_bin: &str, model_bin: &str, model_name: &str, repoexec: bool) -> std::path::PathBuf {
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
                _ => reader.resolve_payload(e).map(|b| String::from_utf8_lossy(&b).into_owned()).unwrap_or_default(),
            };
            (e.kind, p)
        })
        .collect()
}

/// Honest work: verify-then-submit, correct answer. The verifier audits the
/// recorded evidence and lets it pass - exactly one verifier round.
#[test]
fn honest_work_passes_with_one_verifier_round() {
    let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let dir = tempfile::tempdir().unwrap();
    let log = tempfile::tempdir().unwrap();
    let ws = git_ws(dir.path());
    std::env::set_var("HS_SWE_WORKSPACE", &ws);
    let answer = log.path().join("work").join("task-21").join("answer.txt");
    let diff = "```diff\\n--- a/code.txt\\n+++ b/code.txt\\n@@ -1 +1 @@\\n-broken\\n+fixed\\n```";
    let script = dir.path().join("script.jsonl");
    std::fs::write(&script, format!(
        "{{\"tool\":\"repo.exec\",\"args\":{{\"command\":\"cat code.txt\",\"diff\":\"{diff}\"}}}}\n{{\"tool\":\"answer.write\",\"args\":{{\"path\":\"{}\",\"content\":\"TOKEN-21-SECRET\"}}}}",
        answer.display())).unwrap();
    std::env::set_var("HS_VF_SCRIPT", &script);
    let config = config_with(dir.path(), CHECKER, VFMODEL, "vfmodel", true);
    let kernel = hs_kernel::Kernel::load(&config).unwrap();
    let mut l = InnerLoop::new(kernel, log.path(), true, 5).unwrap();
    let r = l.run_mission("task-21").unwrap();
    assert!(r.passed, "honest verified work passes: {r:?}");
    assert_eq!(r.steps, 2, "verify, submit: {r:?}");
    let ev = events_of(log.path(), r.stream_id);
    let verdicts: Vec<&(EventKind, String)> = ev.iter()
        .filter(|(k, p)| *k == EventKind::Feedback && p.contains("verifier"))
        .collect();
    assert_eq!(verdicts.len(), 1, "exactly one verifier round: {verdicts:?}");
    assert!(verdicts[0].1.contains("not_refuted"), "clean verdict: {}", verdicts[0].1);
    assert!(ev.iter().any(|(k, p)| *k == EventKind::ModelCall && p.contains("ADVERSARIAL VERIFIER")),
        "the verifier call is on the audit stream");
}

/// Fabricated claim / unverified submission: answer-only mission (no
/// repo.exec available). The verifier refutes every round; after 3 rounds
/// the cap bites: verifier_ratchet is booked and the checker verdict stands.
#[test]
fn fabricated_claim_is_refuted_until_the_ratchet_cap() {
    let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let dir = tempfile::tempdir().unwrap();
    let log = tempfile::tempdir().unwrap();
    let answer = log.path().join("work").join("task-22").join("answer.txt");
    let script = dir.path().join("script.jsonl");
    std::fs::write(&script, format!(
        "{{\"tool\":\"answer.write\",\"args\":{{\"path\":\"{}\",\"content\":\"TOKEN-22-SECRET\"}}}}",
        answer.display())).unwrap();
    std::env::set_var("HS_VF_SCRIPT", &script);
    let config = config_with(dir.path(), CHECKER, VFMODEL, "vfmodel", false);
    let kernel = hs_kernel::Kernel::load(&config).unwrap();
    let mut l = InnerLoop::new(kernel, log.path(), true, 6).unwrap();
    let r = l.run_mission("task-22").unwrap();
    assert!(r.passed, "cap reached: the checker verdict stands, mission resolves: {r:?}");
    let ev = events_of(log.path(), r.stream_id);
    let refusals = ev.iter()
        .filter(|(k, p)| *k == EventKind::Feedback && p.contains("verifier") && p.contains("refuted"))
        .count();
    assert_eq!(refusals, 3, "three refuted rounds before the cap: {refusals}");
    assert!(ev.iter().any(|(k, p)| *k == EventKind::Feedback && p.contains("verifier_ratchet")),
        "ratchet event booked");
    let prompts: Vec<String> = ev.iter()
        .filter(|(k, _)| *k == EventKind::ModelCall)
        .filter_map(|(_, p)| serde_json::from_str::<serde_json::Value>(p).ok())
        .map(|v| hs_loop::msgfmt::prompt_view(&v))
        .collect();
    assert!(prompts.iter().any(|p| p.contains("VERIFIER REFUTED")),
        "the refusal reaches the next prompt as feedback");
}

/// Hacked evidence: a LYING checker waves a wrong answer through; the
/// verifier is the second wall and sends it back. The repair then passes.
#[test]
fn lying_checker_green_is_vetoed_when_the_answer_is_wrong() {
    let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let dir = tempfile::tempdir().unwrap();
    let log = tempfile::tempdir().unwrap();
    let ws = git_ws(dir.path());
    std::env::set_var("HS_SWE_WORKSPACE", &ws);
    let answer = log.path().join("work").join("task-23").join("answer.txt");
    let diff = "```diff\\n--- a/code.txt\\n+++ b/code.txt\\n@@ -1 +1 @@\\n-broken\\n+fixed\\n```";
    let script = dir.path().join("script.jsonl");
    std::fs::write(&script, format!(
        "{{\"tool\":\"repo.exec\",\"args\":{{\"command\":\"cat code.txt\",\"diff\":\"{diff}\"}}}}\n{{\"tool\":\"answer.write\",\"args\":{{\"path\":\"{}\",\"content\":\"HACKED\"}}}}\n{{\"tool\":\"answer.write\",\"args\":{{\"path\":\"{}\",\"content\":\"TOKEN-23-SECRET\"}}}}",
        answer.display(), answer.display())).unwrap();
    std::env::set_var("HS_VF_SCRIPT", &script);
    let config = config_with(dir.path(), LIECHECKER, VFMODEL, "vfmodel", true);
    let kernel = hs_kernel::Kernel::load(&config).unwrap();
    let mut l = InnerLoop::new(kernel, log.path(), true, 6).unwrap();
    let r = l.run_mission("task-23").unwrap();
    assert!(r.passed, "mission passes only after the real fix: {r:?}");
    assert_eq!(r.steps, 3, "verify, hacked submit vetoed, real submit: {r:?}");
    let ev = events_of(log.path(), r.stream_id);
    let verdicts: Vec<&String> = ev.iter()
        .filter(|(k, p)| *k == EventKind::Feedback && p.contains("verifier"))
        .map(|(_, p)| p)
        .collect();
    assert_eq!(verdicts.len(), 2, "two verifier rounds: {verdicts:?}");
    assert!(verdicts[0].contains("refuted") && !verdicts[0].contains("not_refuted"),
        "round 1 refutes the hacked submission: {}", verdicts[0]);
    assert!(verdicts[1].contains("not_refuted"), "round 2 accepts the real fix: {}", verdicts[1]);
    // and the accepted answer is the real one
    assert_eq!(std::fs::read_to_string(&answer).unwrap(), "TOKEN-23-SECRET");
}

/// A malfunctioning verifier never blocks: an unparseable verdict books
/// verifier_error and the checker verdict stands.
#[test]
fn malformed_verdict_books_error_and_the_checker_stands() {
    let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let dir = tempfile::tempdir().unwrap();
    let log = tempfile::tempdir().unwrap();
    let answer = log.path().join("work").join("task-20").join("answer.txt");
    let script = dir.path().join("script.jsonl");
    std::fs::write(&script, format!(
        "{{\"tool\":\"answer.write\",\"args\":{{\"path\":\"{}\",\"content\":\"TOKEN-20-SECRET\"}}}}",
        answer.display())).unwrap();
    std::env::set_var("HS_SEQMODEL_SCRIPT", &script);
    let config = config_with(dir.path(), CHECKER, SCRIPTED, "scripted", false);
    let kernel = hs_kernel::Kernel::load(&config).unwrap();
    let mut l = InnerLoop::new(kernel, log.path(), true, 4).unwrap();
    let r = l.run_mission("task-20").unwrap();
    assert!(r.passed, "a broken verifier cannot block good work: {r:?}");
    let ev = events_of(log.path(), r.stream_id);
    assert!(ev.iter().any(|(k, p)| *k == EventKind::Feedback && p.contains("verifier_error")),
        "verifier_error booked");
}
