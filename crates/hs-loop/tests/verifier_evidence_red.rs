//! D11 RED (run-of-record 2026-09-09, stream afe9fbf3): the ledger's
//! evidence render kept only the last 160 whitespace-collapsed chars of
//! stdout and dropped stderr entirely (ledger.rs `output_tail`). Diff-loop
//! verification is silent on success, so the only bytes at the tail were
//! the model's own prose banner - "ALL CHECKS PASSED in both modes"
//! (verifier round 1), "ALL TRANSCRIPT CHECKS PASSED" (round 3) - which the
//! verifier's own prompt defines as fabricated ("a prose claim of test
//! output with no recorded run is fabricated: refute"). Three rounds
//! refuted blocking=unverifiable; the mission closed `ratchet_capped` on a
//! deliverable the sealed held-out grader scores 9/9. Evidence rows must
//! carry the exit code, stderr, byte counts, and enough stdout to show the
//! mechanical per-check lines; truncation must be marked, not silent.

use hs_core::{EventKind, Payload};
use hs_loop::*;

const ANSWER: &str = env!("CARGO_BIN_EXE_hs-plugin-answer");
const CHECKER: &str = env!("CARGO_BIN_EXE_hs-plugin-checker");
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

/// The banner trap, unit level. Run4's verification runs printed mechanical
/// per-check lines first and a prose banner last; stderr carried the
/// negative-test evidence. The row must render all of it, byte-counted.
#[test]
fn evidence_row_carries_exit_stderr_bytes_and_mechanical_lines() {
    let mut led = ledger::Ledger::default();
    let stdout = format!(
        "hello diff exit=0\nprecedence diff exit=0\nconcat diff exit=0\n{}\nALL DIFFS CLEAN",
        "banner noise ".repeat(40)
    );
    let stderr = "error: undefined variable `n` (line 3, column 5)";
    led.apply_tool_call(
        7,
        "repo.exec",
        &serde_json::json!({"command": "for t in hello precedence concat; do diff -u tests/$t.expected <(python3 pocket.py tests/$t.pock); echo \"$t diff exit=$?\"; done"}),
        &serde_json::json!({"applied": true, "exit_code": 0, "stdout": stdout, "stderr": stderr}),
    );
    let s = led.summary();
    assert!(
        s.contains("exit=0"),
        "the exit code is the mechanical verdict: {s}"
    );
    assert!(
        s.contains("undefined variable"),
        "stderr is load-bearing evidence (negative tests) - dropping it hides the run: {s}"
    );
    assert!(
        s.contains("precedence diff exit=0"),
        "mechanical per-check lines survive the bound (the banner must not push them out): {s}"
    );
    let collapsed_stdout = stdout.split_whitespace().collect::<Vec<_>>().join(" ");
    assert!(
        s.contains(&format!("stdout={}B", collapsed_stdout.len())),
        "byte counts mark exactly what was kept and dropped: {s}"
    );
    assert!(
        s.contains(&format!("stderr={}B", stderr.len())),
        "stderr is byte-counted too: {s}"
    );
}

/// Run-of-record replay, RED-first: verify with a diff loop whose stdout
/// ends in a long prose banner (the run4 shape), then submit. The
/// verifier's LEDGER section must carry the mechanical per-check lines -
/// and the same submission is accepted on ONE round, no ratchet fallback.
#[test]
fn verifier_sees_mechanical_evidence_and_accepts_without_ratchet() {
    let _g = LOCK.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let dir = tempfile::tempdir().unwrap();
    let log = tempfile::tempdir().unwrap();
    let ws = git_ws(dir.path());
    // FIXME: Audit that the environment access only happens in single-threaded code.
    unsafe { std::env::set_var("HS_SWE_WORKSPACE", &ws) };
    let answer = log.path().join("work").join("task-17").join("answer.txt");
    let diff = "```diff\\n--- a/code.txt\\n+++ b/code.txt\\n@@ -1 +1 @@\\n-broken\\n+fixed\\n```";
    // run4 shape: mechanical per-check lines, then a long prose banner.
    // no double quotes: the script line embeds the command raw into JSON.
    let verify = "for t in hello precedence concat; do echo $t diff exit=0; done; for i in 1 2 3 4 5 6 7 8 9 10 11 12 13 14 15 16 17 18 19 20; do printf 'banner noise '; done; echo; echo ALL DIFFS CLEAN";
    let script = dir.path().join("script.jsonl");
    std::fs::write(&script, format!(
        "{{\"tool\":\"repo.exec\",\"args\":{{\"command\":\"{verify}\",\"diff\":\"{diff}\"}}}}\n{{\"tool\":\"answer.write\",\"args\":{{\"path\":\"{}\",\"content\":\"TOKEN-17-SECRET\"}}}}",
        answer.display())).unwrap();
    // FIXME: Audit that the environment access only happens in single-threaded code.
    unsafe { std::env::set_var("HS_VF_SCRIPT", &script) };
    let config = config_with(dir.path(), CHECKER, VFMODEL, "vfmodel", true);
    let kernel = hs_kernel::Kernel::load(&config).unwrap();
    let mut l = InnerLoop::new(kernel, log.path(), true, 5).unwrap();
    let r = l.run_mission("task-17").unwrap();
    assert!(r.passed, "honest verified work passes: {r:?}");
    assert_eq!(r.steps, 2, "verify, submit: {r:?}");
    let ev = events_of(log.path(), r.stream_id);
    let vprompts: Vec<String> = ev
        .iter()
        .filter(|(k, _)| *k == EventKind::ModelCall)
        .filter_map(|(_, p)| serde_json::from_str::<serde_json::Value>(p).ok())
        .map(|v| hs_loop::msgfmt::prompt_view(&v))
        .filter(|p| p.contains("ADVERSARIAL VERIFIER"))
        .collect();
    assert_eq!(
        vprompts.len(),
        1,
        "accepted on exactly one verifier round: {vprompts:?}"
    );
    let vp = &vprompts[0];
    assert!(
        vp.contains("precedence diff exit=0"),
        "the verifier audits mechanical per-check lines, not the banner: {vp}"
    );
    assert!(
        vp.contains("exit=0"),
        "exit codes render as mechanical evidence: {vp}"
    );
    let verdicts: Vec<&String> = ev
        .iter()
        .filter(|(k, p)| *k == EventKind::Feedback && p.contains("verifier"))
        .map(|(_, p)| p)
        .collect();
    assert!(
        verdicts.iter().any(|v| v.contains("not_refuted")),
        "the same submission is accepted: {verdicts:?}"
    );
    assert!(
        !ev.iter().any(|(_, p)| p.contains("verifier_ratchet")),
        "no ratchet fallback on honest, mechanically-evidenced work"
    );
}
