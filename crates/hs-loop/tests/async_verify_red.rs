//! Async verifier seam (gate-8 waste-only redesign, Eric 2026-09-06):
//! the agent keeps working while the adversarial audit runs; a veto
//! restores the audited snapshot; identical resubmissions replay the
//! recorded verdict instead of re-burning a max-effort call.
//!
//! Promotion gate: these tests must pass AND the 10-mission re-run must
//! reproduce today's verdicts round-by-round before async becomes default.

use hs_core::{EventKind, Payload};
use hs_loop::*;

const ANSWER: &str = env!("CARGO_BIN_EXE_hs-plugin-answer");
const CHECKER: &str = env!("CARGO_BIN_EXE_hs-plugin-checker");
const LIECHECKER: &str = env!("CARGO_BIN_EXE_hs-plugin-liechecker");
const REPOEXEC: &str = env!("CARGO_BIN_EXE_hs-plugin-repoexec");
const VFMODEL: &str = env!("CARGO_BIN_EXE_hs-plugin-vfmodel");

// HS_VF_SCRIPT / HS_VF_LOG / HS_VF_VERIFIER_SLEEP_MS / HS_SWE_WORKSPACE are
// process-global: serialize.
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

fn config_with(dir: &std::path::Path, checker_bin: &str) -> std::path::PathBuf {
    let c = format!(
        r#"
[[tools]]
name = "answer.write"
command = ["{ANSWER}"]
subjects = ["*"]

[[tools]]
name = "checker.run"
command = ["{checker_bin}"]
subjects = ["*"]

[[tools]]
name = "repo.exec"
command = ["{REPOEXEC}"]
subjects = ["*"]

[[models]]
name = "vfmodel"
command = ["{VFMODEL}"]
default = true
"#
    );
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

fn verifier_calls() -> Vec<serde_json::Value> {
    let p = std::env::var("HS_VF_LOG").unwrap();
    std::fs::read_to_string(p)
        .unwrap_or_default()
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| serde_json::from_str(l).unwrap())
        .filter(|e: &serde_json::Value| e["kind"] == "verifier")
        .collect()
}

const DIFF: &str = "```diff\\n--- a/code.txt\\n+++ b/code.txt\\n@@ -1 +1 @@\\n-broken\\n+fixed\\n```";

fn write_script(dir: &std::path::Path, lines: &[String]) {
    std::fs::write(dir.join("script.jsonl"), lines.join("\n")).unwrap();
    std::env::set_var("HS_VF_SCRIPT", dir.join("script.jsonl"));
}

fn setup(dir: &std::path::Path, log: &std::path::Path, sleep_ms: Option<u64>) -> (std::path::PathBuf, std::path::PathBuf) {
    let ws = git_ws(dir);
    std::env::set_var("HS_SWE_WORKSPACE", &ws);
    match sleep_ms {
        Some(ms) => std::env::set_var("HS_VF_VERIFIER_SLEEP_MS", ms.to_string()),
        None => std::env::remove_var("HS_VF_VERIFIER_SLEEP_MS"),
    }
    let vflog = dir.join("vf.jsonl");
    std::env::set_var("HS_VF_LOG", &vflog);
    (ws, log.join("work"))
}

/// T1 (fix 3, sync path): the verifier ModelCall event carries real
/// latency + token usage - today it books lat=0 / no usage, which hid the
/// 48% verifier wall share.
#[test]
fn t1_verifier_modelcall_books_latency_and_usage() {
    let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let dir = tempfile::tempdir().unwrap();
    let log = tempfile::tempdir().unwrap();
    let (_ws, work) = setup(dir.path(), log.path(), Some(50));
    let answer = work.join("task-21").join("answer.txt");
    write_script(dir.path(), &[
        format!("{{\"tool\":\"repo.exec\",\"args\":{{\"command\":\"cat code.txt\",\"diff\":\"{DIFF}\"}}}}"),
        format!("{{\"tool\":\"answer.write\",\"args\":{{\"path\":\"{}\",\"content\":\"TOKEN-21-SECRET\"}}}}", answer.display()),
    ]);
    let config = config_with(dir.path(), CHECKER);
    let kernel = hs_kernel::Kernel::load(&config).unwrap();
    let mut l = InnerLoop::new(kernel, log.path(), true, 5).unwrap();
    let r = l.run_mission("task-21").unwrap();
    assert!(r.passed, "honest verified work passes: {r:?}");
    let ev = events_of(log.path(), r.stream_id);
    let vcalls: Vec<&(EventKind, String)> = ev.iter()
        .filter(|(k, p)| *k == EventKind::ModelCall && p.contains("\"role\":\"verifier\""))
        .collect();
    assert_eq!(vcalls.len(), 1, "exactly one verifier round: {vcalls:?}");
    let body: serde_json::Value = serde_json::from_str(&vcalls[0].1).unwrap();
    assert!(body["input_tokens"].as_u64().unwrap_or(0) > 0, "verifier call books input tokens: {body}");
    assert!(body["latency_ms"].as_u64().unwrap_or(0) >= 50, "verifier call books real latency: {body}");
}

/// T2 (fix 1): the agent keeps working while the audit runs; a clean
/// verdict banks the AUDITED state - speculative edits are reverted.
#[test]
fn t2_agent_works_through_the_audit_and_banks_the_audited_state() {
    let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let dir = tempfile::tempdir().unwrap();
    let log = tempfile::tempdir().unwrap();
    let (ws, work) = setup(dir.path(), log.path(), Some(800));
    let answer = work.join("task-21").join("answer.txt");
    write_script(dir.path(), &[
        format!("{{\"tool\":\"repo.exec\",\"args\":{{\"command\":\"cat code.txt\",\"diff\":\"{DIFF}\"}}}}"),
        format!("{{\"tool\":\"answer.write\",\"args\":{{\"path\":\"{}\",\"content\":\"TOKEN-21-SECRET\"}}}}", answer.display()),
        format!("{{\"tool\":\"repo.exec\",\"args\":{{\"command\":\"touch spec.txt\",\"diff\":\"{DIFF}\"}}}}"),
        "{\"tool\":\"repo.exec\",\"args\":{\"command\":\"cat code.txt\"}}".to_string(),
    ]);
    let config = config_with(dir.path(), CHECKER);
    let kernel = hs_kernel::Kernel::load(&config).unwrap();
    let mut l = InnerLoop::new(kernel, log.path(), true, 60).unwrap();
    l.set_async_verify(&ws);
    let r = l.run_mission("task-21").unwrap();
    assert!(r.passed, "verified work passes in async mode: {r:?}");
    let ev = events_of(log.path(), r.stream_id);
    let submit_pos = ev.iter().position(|(_, p)| p.contains("answer.write") && p.contains("TOKEN-21-SECRET")).unwrap_or(usize::MAX);
    let spec_pos = ev.iter().position(|(k, p)| *k == EventKind::ToolCall && p.contains("spec.txt")).unwrap_or(usize::MAX);
    let verdict_pos = ev.iter().position(|(k, p)| *k == EventKind::Feedback && p.contains("\"verdict\":\"not_refuted\"")).unwrap_or(usize::MAX);
    assert!(submit_pos < spec_pos && spec_pos < verdict_pos,
        "agent works through the audit window: submit@{submit_pos} spec@{spec_pos} verdict@{verdict_pos}");
    assert!(verdict_pos < usize::MAX, "a verdict arrived");
    assert!(!ws.join("spec.txt").exists(), "banked state is the audited state, not the speculative one");
}

/// T3 (fix 1, veto path): a refuted round restores the audited snapshot,
/// hands back the same feedback as the sync path, and round 2 audits with
/// round 1's gaps - strictly after round 1 finished.
#[test]
fn t3_veto_restores_snapshot_and_rounds_stay_sequential() {
    let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let dir = tempfile::tempdir().unwrap();
    let log = tempfile::tempdir().unwrap();
    let (ws, work) = setup(dir.path(), log.path(), Some(1500));
    let answer = work.join("task-21").join("answer.txt");
    // liechecker: every submit is checker-green, so the VERIFIER is what
    // judges. Round 1 refutes (no recorded test run at submit time); the
    // last line cycles so the agent keeps resubmitting the fixed answer.
    write_script(dir.path(), &[
        format!("{{\"tool\":\"repo.exec\",\"args\":{{\"command\":\"cat code.txt\",\"diff\":\"{DIFF}\"}}}}"),
        format!("{{\"tool\":\"answer.write\",\"args\":{{\"path\":\"{}\",\"content\":\"WRONG\"}}}}", answer.display()),
        format!("{{\"tool\":\"repo.exec\",\"args\":{{\"command\":\"touch spec2.txt\",\"diff\":\"{DIFF}\"}}}}"),
        format!("{{\"tool\":\"answer.write\",\"args\":{{\"path\":\"{}\",\"content\":\"TOKEN-21-SECRET\"}}}}", answer.display()),
    ]);
    let config = config_with(dir.path(), LIECHECKER);
    let kernel = hs_kernel::Kernel::load(&config).unwrap();
    let mut l = InnerLoop::new(kernel, log.path(), true, 60).unwrap();
    l.set_async_verify(&ws);
    let r = l.run_mission("task-21").unwrap();
    assert!(r.passed, "repaired work passes: {r:?}");
    let ev = events_of(log.path(), r.stream_id);
    let refute = ev.iter().find(|(k, p)| *k == EventKind::Feedback && p.contains("\"verdict\":\"refuted\""))
        .expect("a refuted round happened");
    assert!(refute.1.contains("\"round\":1"), "round 1 refuted: {}", refute.1);
    assert!(ev.iter().any(|(k, p)| *k == EventKind::Feedback && p.contains("verifier_restore")),
        "the audited-snapshot restore is recorded");
    let vcalls = verifier_calls();
    assert_eq!(vcalls.len(), 2, "two real verifier rounds: {vcalls:?}");
    assert!(vcalls[0]["end_ms"].as_u64().unwrap() <= vcalls[1]["start_ms"].as_u64().unwrap(),
        "round 2 starts after round 1 ends: {vcalls:?}");
    assert!(vcalls[1]["prompt"].as_str().unwrap().contains("answer does not deliver the objective"),
        "round 2 carries round 1's gap in PRIOR_GAPS");
    assert!(!ws.join("spec2.txt").exists(), "veto wiped the speculative edit");
}

/// T4 (fix 2): a resubmission identical in artifact + answer + evidence +
/// gaps replays the recorded verdict - no model call is burned. Round 2 of
/// an identical resubmission is NOT cached (its PRIOR_GAPS changed - the
/// anti-ratchet asks a fresh question); round 3 IS (every byte the audit
/// consults is identical to round 2).
#[test]
fn t4_identical_resubmission_replays_the_recorded_verdict() {
    let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let dir = tempfile::tempdir().unwrap();
    let log = tempfile::tempdir().unwrap();
    let (ws, work) = setup(dir.path(), log.path(), None);
    let answer = work.join("task-21").join("answer.txt");
    write_script(dir.path(), &[
        format!("{{\"tool\":\"repo.exec\",\"args\":{{\"command\":\"cat code.txt\",\"diff\":\"{DIFF}\"}}}}"),
        format!("{{\"tool\":\"answer.write\",\"args\":{{\"path\":\"{}\",\"content\":\"WRONG\"}}}}", answer.display()),
    ]);
    let config = config_with(dir.path(), LIECHECKER);
    let kernel = hs_kernel::Kernel::load(&config).unwrap();
    let mut l = InnerLoop::new(kernel, log.path(), true, 60).unwrap();
    l.set_async_verify(&ws);
    let r = l.run_mission("task-21").unwrap();
    assert!(r.passed, "the checker verdict stands after the cap: {r:?}");
    let ev = events_of(log.path(), r.stream_id);
    assert!(ev.iter().any(|(k, p)| *k == EventKind::Feedback && p.contains("verifier_cache_hit")),
        "the identical resubmission is served from the verdict cache");
    assert!(ev.iter().any(|(k, p)| *k == EventKind::Feedback && p.contains("verifier_ratchet")),
        "the ratchet ends the spiral");
    assert_eq!(verifier_calls().len(), 2, "cache kills the redundant call: {:?}", verifier_calls());
}

/// T5 (ratchet intact under async): refuted rounds still exhaust to
/// verifier_ratchet and the checker verdict stands.
#[test]
fn t5_ratchet_cap_holds_in_async_mode() {
    let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let dir = tempfile::tempdir().unwrap();
    let log = tempfile::tempdir().unwrap();
    let (ws, work) = setup(dir.path(), log.path(), None);
    let answer = work.join("task-22").join("answer.txt");
    write_script(dir.path(), &[
        format!("{{\"tool\":\"repo.exec\",\"args\":{{\"command\":\"cat code.txt\",\"diff\":\"{DIFF}\"}}}}"),
        format!("{{\"tool\":\"answer.write\",\"args\":{{\"path\":\"{}\",\"content\":\"WRONG\"}}}}", answer.display()),
    ]);
    let config = config_with(dir.path(), LIECHECKER);
    let kernel = hs_kernel::Kernel::load(&config).unwrap();
    let mut l = InnerLoop::new(kernel, log.path(), true, 60).unwrap();
    l.set_async_verify(&ws);
    let r = l.run_mission("task-22").unwrap();
    assert!(r.passed, "checker green stands after the cap: {r:?}");
    let ev = events_of(log.path(), r.stream_id);
    assert!(ev.iter().any(|(k, p)| *k == EventKind::Feedback && p.contains("verifier_ratchet")),
        "the ratchet still bites after 3 rounds");
}
