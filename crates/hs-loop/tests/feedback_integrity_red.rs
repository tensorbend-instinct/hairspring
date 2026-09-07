//! Feedback-integrity RED (Eric 2026-09-06: "find and fix any/all other
//! such inconsistencies" of the stonewall class - anywhere the harness
//! tells the agent one thing while meaning another, buries feedback, or
//! lets a green checkmark beat a refutation). Each test names the finding
//! it pins; every one is behavior-driven, no mocks of the loop itself.

use hs_core::{EventKind, Payload};
use hs_loop::*;

const ANSWER: &str = env!("CARGO_BIN_EXE_hs-plugin-answer");
const CHECKER: &str = env!("CARGO_BIN_EXE_hs-plugin-checker");
const LIECHECKER: &str = env!("CARGO_BIN_EXE_hs-plugin-liechecker");
const REPOEXEC: &str = env!("CARGO_BIN_EXE_hs-plugin-repoexec");
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

fn config_with(dir: &std::path::Path, checker_bin: &str, repoexec: bool) -> std::path::PathBuf {
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
name = "vfmodel"
command = ["{VFMODEL}"]
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

fn prompts_of(ev: &[(EventKind, String)]) -> Vec<String> {
    ev.iter()
        .filter(|(k, _)| *k == EventKind::ModelCall)
        .filter_map(|(_, p)| serde_json::from_str::<serde_json::Value>(p).ok())
        .filter(|v| v["role"].as_str() != Some("verifier"))
        .map(|v| hs_loop::msgfmt::prompt_view(&v))
        .collect()
}

/// Drive a mission that verifies, submits a WRONG answer (liechecker waves
/// it through), gets refuted, then re-issues the same repo.exec call over
/// and over - the 17092 stonewall shape in miniature.
fn refute_then_doom(
    mission: &str,
) -> (
    Vec<(EventKind, String)>,
    tempfile::TempDir,
    tempfile::TempDir,
) {
    let dir = tempfile::tempdir().unwrap();
    let log = tempfile::tempdir().unwrap();
    let ws = git_ws(dir.path());
    std::env::set_var("HS_SWE_WORKSPACE", &ws);
    let answer = log.path().join("work").join(mission).join("answer.txt");
    let diff = "```diff\\n--- a/code.txt\\n+++ b/code.txt\\n@@ -1 +1 @@\\n-broken\\n+fixed\\n```";
    let script = dir.path().join("script.jsonl");
    std::fs::write(
        &script,
        format!(
        "{{\"tool\":\"repo.exec\",\"args\":{{\"command\":\"cat code.txt\",\"diff\":\"{diff}\"}}}}\n\
         {{\"tool\":\"answer.write\",\"args\":{{\"path\":\"{}\",\"content\":\"HACKED\"}}}}\n\
         {{\"tool\":\"repo.exec\",\"args\":{{\"command\":\"cat code.txt\",\"diff\":\"{diff}\"}}}}\n\
         {{\"tool\":\"repo.exec\",\"args\":{{\"command\":\"cat code.txt\",\"diff\":\"{diff}\"}}}}\n\
         {{\"tool\":\"repo.exec\",\"args\":{{\"command\":\"cat code.txt\",\"diff\":\"{diff}\"}}}}",
        answer.display()),
    )
    .unwrap();
    std::env::set_var("HS_VF_SCRIPT", &script);
    let config = config_with(dir.path(), LIECHECKER, true);
    let kernel = hs_kernel::Kernel::load(&config).unwrap();
    let mut l = InnerLoop::new(kernel, log.path(), true, 6).unwrap();
    let r = l.run_mission(mission).unwrap();
    assert!(!r.passed, "the stonewalling agent never passes: {r:?}");
    (events_of(log.path(), r.stream_id), dir, log)
}

/// Finding 4: the refute injection advertised "round N/3" to the agent -
/// the exact stonewall budget. The agent-facing text must carry the verdict
/// and its findings, never the remaining-round count.
#[test]
fn refute_feedback_carries_no_round_counter() {
    let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let (ev, _d, _l) = refute_then_doom("task-41");
    let prompts = prompts_of(&ev);
    let with_refute: Vec<&String> = prompts
        .iter()
        .filter(|p| p.contains("VERIFIER REFUTED"))
        .collect();
    assert!(
        !with_refute.is_empty(),
        "the refute reaches the prompt: {}",
        prompts.len()
    );
    for p in with_refute {
        assert!(
            !p.contains("(round "),
            "no stonewall budget in agent-facing text: {p}"
        );
    }
}

/// Finding 1: the doom-loop nudge offered "or verify and submit" even when
/// a refuted verdict was outstanding - an escape hatch that converts the
/// anti-stonewall tripwire into stonewall instructions. After a refute the
/// nudge must point at the findings, never at submission.
#[test]
fn doom_loop_after_refute_offers_no_submit_hatch() {
    let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let (ev, _d, _l) = refute_then_doom("task-42");
    let notes: Vec<&String> = ev
        .iter()
        .filter(|(k, p)| *k == EventKind::ContextInject && p.contains("doom_loop"))
        .map(|(_, p)| p)
        .collect();
    assert!(
        !notes.is_empty(),
        "the doom loop detector fires on the repeated call: {ev:?}"
    );
    for n in &notes {
        assert!(
            !n.contains("verify and submit"),
            "no submit hatch while a refute is outstanding: {n}"
        );
        assert!(
            n.contains("REFUTED"),
            "the nudge names the outstanding refute: {n}"
        );
    }
}

/// Finding 5: the veto rode at the TAIL of an 8k-char message, behind the
/// full artifact. Feedback leads the state tail: FEEDBACK before ARTIFACT.
#[test]
fn feedback_leads_artifact_in_tail() {
    let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let (ev, _d, _l) = refute_then_doom("task-43");
    let prompts = prompts_of(&ev);
    // 2026-09-07: the artifact line is now labeled ("ARTIFACT (the graded
    // answer file ...):") - the load-bearing property is FEEDBACK *before*
    // ARTIFACT, not the bare literal.
    let post_veto: Vec<&String> = prompts
        .iter()
        .filter(|p| p.contains("VERIFIER REFUTED") && p.contains("ARTIFACT"))
        .collect();
    assert!(
        !post_veto.is_empty(),
        "a post-veto prompt with an artifact exists"
    );
    for p in post_veto {
        let f = p.find("FEEDBACK:").unwrap();
        let a = p.find("ARTIFACT").unwrap();
        assert!(
            f < a,
            "feedback must lead, not trail the artifact: FEEDBACK@{f} ARTIFACT@{a}"
        );
    }
}

/// Finding 7: the verifier is told "a prose claim of test output with no
/// recorded run is fabricated: refute" - but the ledger recorded only
/// command + verdict, never output, so the demand was unsatisfiable. The
/// ledger keeps a bounded output tail per run.
#[test]
fn ledger_records_bounded_output_tail() {
    let mut led = ledger::Ledger::default();
    led.apply_tool_call(
        7,
        "repo.exec",
        &serde_json::json!({"command": "python3 -m pytest tests/ -q"}),
        &serde_json::json!({"applied": true, "exit_code": 0, "stdout": "========================= 69 passed in 12.34s ========================="}),
    );
    let s = led.summary();
    assert!(
        s.contains("69 passed"),
        "recorded run carries its output tail: {s}"
    );
}

/// Finding 8: after a refute the workspace is RESTORED to the audited
/// snapshot, but the ledger kept rendering pre-restore runs as current
/// truth under an "always current" heading. Runs recorded at or before the
/// restore point render marked.
#[test]
fn ledger_marks_pre_restore_runs() {
    let mut led = ledger::Ledger::default();
    let run = |_seq: u64| {
        (
            serde_json::json!({"command": "pytest -q"}),
            serde_json::json!({"applied": true, "exit_code": 0, "stdout": "1 passed"}),
        )
    };
    let (a, r) = run(3);
    led.apply_tool_call(3, "repo.exec", &a, &r);
    led.note_restore(5);
    let (a2, r2) = run(6);
    led.apply_tool_call(6, "repo.exec", &a2, &r2);
    let s = led.summary();
    assert!(s.contains("seq3(pre-restore)"), "stale run marked: {s}");
    assert!(
        !s.contains("seq6(pre-restore)"),
        "post-restore run unmarked: {s}"
    );
}

/// Finding 6: the mission prompt said "If you get FEEDBACK, repair and
/// continue" (advisory) next to the imperative checker nudge, and nowhere
/// told the agent an identical resubmit after a refute is replayed without
/// new scrutiny - the agent could rationally believe resubmission gets
/// fresh eyes. The template makes repair binding and names the replay.
#[test]
fn mission_prompt_makes_feedback_repair_binding() {
    let args = sweprompt::PromptArgs {
        ws: "/ws".into(),
        problem_statement: "p".into(),
        fail_to_pass: vec!["pytest -q".into()],
        repo_layout: "".into(),
        nudge: "".into(),
        answer_path: "/a".into(),
        orientation: "".into(),
        mcp_tools: String::new(),
    };
    let p = sweprompt::build_mission_prompt(None, &args);
    assert!(
        p.contains("earns another refutation"),
        "the prompt names the re-refute: {p}"
    );
    assert!(
        !p.contains("verdict cache"),
        "no claim about machinery the tree does not have: {p}"
    );
    assert!(
        !p.contains("repair and continue"),
        "no advisory dodge left: {p}"
    );
}

const SCRIPTED: &str = env!("CARGO_BIN_EXE_hs-plugin-scripted");

fn config_for(
    dir: &std::path::Path,
    checker_bin: &str,
    model_bin: &str,
    model_name: &str,
) -> std::path::PathBuf {
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

[[models]]
name = "{model_name}"
command = ["{model_bin}"]
default = true
"#
    );
    let p = dir.join("hairspring.toml");
    std::fs::write(&p, c).unwrap();
    p
}

/// Finding 2: the ratchet cap banked a PASS byte-identical to a verified
/// one - result.json could not tell "refuted 3x then the cap freed it"
/// from "audited and accepted". The result carries an outcome label.
#[test]
fn ratchet_cap_pass_is_labeled_ratchet_capped() {
    let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let dir = tempfile::tempdir().unwrap();
    let log = tempfile::tempdir().unwrap();
    let answer = log.path().join("work").join("task-12").join("answer.txt");
    let script = dir.path().join("script.jsonl");
    std::fs::write(&script, format!(
        "{{\"tool\":\"answer.write\",\"args\":{{\"path\":\"{}\",\"content\":\"TOKEN-12-SECRET\"}}}}",
        answer.display())).unwrap();
    std::env::set_var("HS_VF_SCRIPT", &script);
    let config = config_for(dir.path(), CHECKER, VFMODEL, "vfmodel");
    let kernel = hs_kernel::Kernel::load(&config).unwrap();
    let mut l = InnerLoop::new(kernel, log.path(), true, 6).unwrap();
    let r = l.run_mission("task-12").unwrap();
    assert!(r.passed, "cap reached: the mission resolves: {r:?}");
    assert_eq!(
        r.outcome, "ratchet_capped",
        "a capped pass is labeled, never silent: {r:?}"
    );
}

/// Finding 3: a verifier malfunction (unparseable verdict, dead worker)
/// banked the same silent PASS. Labeled too.
#[test]
fn malfunction_pass_is_labeled_verifier_malfunction() {
    let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let dir = tempfile::tempdir().unwrap();
    let log = tempfile::tempdir().unwrap();
    let answer = log.path().join("work").join("task-13").join("answer.txt");
    let script = dir.path().join("script.jsonl");
    std::fs::write(&script, format!(
        "{{\"tool\":\"answer.write\",\"args\":{{\"path\":\"{}\",\"content\":\"TOKEN-13-SECRET\"}}}}",
        answer.display())).unwrap();
    std::env::set_var("HS_SEQMODEL_SCRIPT", &script);
    let config = config_for(dir.path(), CHECKER, SCRIPTED, "scripted");
    let kernel = hs_kernel::Kernel::load(&config).unwrap();
    let mut l = InnerLoop::new(kernel, log.path(), true, 4).unwrap();
    let r = l.run_mission("task-13").unwrap();
    assert!(r.passed, "a broken verifier cannot block good work: {r:?}");
    assert_eq!(
        r.outcome, "verifier_malfunction",
        "a malfunction pass is labeled, never silent: {r:?}"
    );
}

/// The normal path: audited and accepted - labeled "verified".
#[test]
fn audited_pass_is_labeled_verified() {
    let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let dir = tempfile::tempdir().unwrap();
    let log = tempfile::tempdir().unwrap();
    let ws = git_ws(dir.path());
    std::env::set_var("HS_SWE_WORKSPACE", &ws);
    let answer = log.path().join("work").join("task-14").join("answer.txt");
    let diff = "```diff\\n--- a/code.txt\\n+++ b/code.txt\\n@@ -1 +1 @@\\n-broken\\n+fixed\\n```";
    let script = dir.path().join("script.jsonl");
    std::fs::write(&script, format!(
        "{{\"tool\":\"repo.exec\",\"args\":{{\"command\":\"cat code.txt\",\"diff\":\"{diff}\"}}}}\n{{\"tool\":\"answer.write\",\"args\":{{\"path\":\"{}\",\"content\":\"TOKEN-14-SECRET\"}}}}",
        answer.display())).unwrap();
    std::env::set_var("HS_VF_SCRIPT", &script);
    let config = config_with(dir.path(), CHECKER, true);
    let kernel = hs_kernel::Kernel::load(&config).unwrap();
    let mut l = InnerLoop::new(kernel, log.path(), true, 5).unwrap();
    let r = l.run_mission("task-14").unwrap();
    assert!(r.passed, "honest verified work passes: {r:?}");
    assert_eq!(r.outcome, "verified", "the audited path is labeled: {r:?}");
}

/// Finding 10: the convergence nudge offered "or state in one line what
/// you will change and how you will verify it" - prose the native
/// protocol forbids (TOOLS: call exactly one per reply, no prose) and the
/// mission's own WORK POLICY bans ("do the work in the current step
/// instead of ending with an offer to do it later"). The nudge must
/// demand verification, not offer a prose exit.
#[test]
fn convergence_nudge_offers_no_prose_exit() {
    let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let dir = tempfile::tempdir().unwrap();
    let log = tempfile::tempdir().unwrap();
    let answer = log.path().join("work").join("task-15").join("answer.txt");
    let script = dir.path().join("script.jsonl");
    std::fs::write(
        &script,
        format!(
            "{{\"tool\":\"answer.write\",\"args\":{{\"path\":\"{}\",\"content\":\"WRONG\"}}}}",
            answer.display()
        ),
    )
    .unwrap();
    std::env::set_var("HS_SEQMODEL_SCRIPT", &script);
    let config = config_for(dir.path(), CHECKER, SCRIPTED, "scripted");
    let kernel = hs_kernel::Kernel::load(&config).unwrap();
    let mut l = InnerLoop::new(kernel, log.path(), true, 4).unwrap();
    let r = l.run_mission("task-15").unwrap();
    assert!(!r.passed, "wrong answer never passes: {r:?}");
    let ev = events_of(log.path(), r.stream_id);
    let prompts = prompts_of(&ev);
    let nudges: Vec<&String> = prompts
        .iter()
        .filter(|p| p.contains("CONVERGENCE:"))
        .collect();
    assert!(
        !nudges.is_empty(),
        "the convergence nudge fires at the half-step with no self-verification"
    );
    for n in nudges {
        assert!(
            !n.contains("or state"),
            "no prose exit the protocol forbids: {n}"
        );
    }
}
