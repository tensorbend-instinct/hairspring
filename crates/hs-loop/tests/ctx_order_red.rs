//! KV-CACHE PREFIX ORDER (spec v4: "KV-cache reuse and model routing. Prefix
//! reuse across steps", called the highest-leverage production metric).
//!
//! The per-step ctx must lay out as a monotonically growing stable prefix:
//! MISSION (stable) -> TRANSCRIPT (append-only tail) -> volatile fields
//! (ATTEMPT / ANSWER_PATH / ARTIFACT / FEEDBACK) last. If any volatile line
//! sits before the transcript, the cached prefix breaks on every step and
//! every call re-prefills from scratch.
//!
//! Falsifiable: reads the ModelCall prompts back from the mission's own
//! event stream and asserts the section order byte-for-byte.

use hs_loop::*;

const ANSWER: &str = env!("CARGO_BIN_EXE_hs-plugin-answer");
const SWECHECK: &str = env!("CARGO_BIN_EXE_hs-plugin-swecheck");
const SWEMODEL: &str = env!("CARGO_BIN_EXE_hs-plugin-swemodel");

#[test]
fn ctx_layout_keeps_stable_prefix_first() {
    let dir = tempfile::tempdir().unwrap();
    let ws = dir.path().join("ws");
    std::fs::create_dir_all(&ws).unwrap();
    std::fs::write(ws.join("code.txt"), "broken\n").unwrap();
    std::fs::write(ws.join("check.sh"), "#!/bin/sh\ngrep -q '^fixed$' code.txt\n").unwrap();
    let git = |args: &[&str]| {
        assert!(std::process::Command::new("git")
            .args(args).current_dir(&ws).status().unwrap().success());
    };
    git(&["init", "-q"]);
    git(&["config", "user.email", "t@t"]);
    git(&["config", "user.name", "t"]);
    git(&["add", "-A"]);
    git(&["commit", "-qm", "base"]);
    std::fs::write(
        dir.path().join("gold.patch"),
        "--- a/code.txt\n+++ b/code.txt\n@@ -1 +1 @@\n-broken\n+fixed\n",
    )
    .unwrap();

    std::env::set_var("HS_SWE_WORKSPACE", &ws);
    std::env::set_var("HS_SWE_F2P", "sh check.sh");
    std::env::set_var("HS_SWE_P2P", "");
    std::env::set_var("HS_SWE_GOLD_PATCH_FILE", dir.path().join("gold.patch"));

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
command = ["{SWECHECK}"]
subjects = ["*"]

[[models]]
name = "swemodel"
command = ["{SWEMODEL}"]
default = true
"#
        ),
    )
    .unwrap();
    let kernel = hs_kernel::Kernel::load(&config).unwrap();
    let log = dir.path().join("log");
    let mut l = InnerLoop::new(kernel, &log, true, 8).unwrap();
    l.set_budget_micros(10_000);
    let sid = l.stream_id();
    let r = l
        .run_mission_full(
            "fixture__ctx-1",
            "MISSION fixture: code.txt must contain the word fixed. \
             Reply with a JSON tool call answer.write whose content is one fenced unified diff.",
        )
        .unwrap();
    assert!(r.passed && r.steps == 2, "fixture must pass in 2 steps");

    // read the prompts back from the stream itself
    let reader = hs_log::StreamReader::open(&log, sid).unwrap();
    let events = reader.events().unwrap();
    let prompts: Vec<String> = events
        .iter()
        .filter(|e| e.kind == hs_core::EventKind::ModelCall)
        .map(|e| {
            let p = String::from_utf8_lossy(&reader.resolve_payload(e).unwrap()).to_string();
            let v: serde_json::Value = serde_json::from_str(&p).unwrap();
            v
        })
        // item 3: verifier calls are a separate role, not mission steps
        .filter(|v| v["role"].as_str() != Some("verifier"))
        .map(|v| hs_loop::msgfmt::prompt_view(&v))
        .collect();
    let step2 = prompts.last().expect("a second-step prompt must exist");
    // structured-messages contract (2026-09-05): history is native
    // assistant/tool pairs, flattened here as "- tool(args) => result"
    // lines; the ordering law is unchanged - stable mission first, history
    // in the middle, volatile state tail last.
    assert!(
        step2.contains("- answer.write("),
        "step 2 must carry the history pair: {step2}"
    );
    let pos = |needle: &str| step2.find(needle).unwrap_or_else(|| panic!("{needle} missing: {step2}"));
    let (m, t, a) = (
        pos("MISSION:"),
        pos("- answer.write("),
        pos("ATTEMPT:"),
    );
    assert!(
        m < t && t < a,
        "stable prefix must come first: MISSION({m}) < history({t}) < state tail({a})"
    );
    assert!(
        !step2.contains("TRANSCRIPT (earlier tool calls):"),
        "no hand-rendered transcript heading in the native world"
    );
}
