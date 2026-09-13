//! Dance #95 D8/D9 (live burn 2026-09-09, realrun2): a REPL mission
//! launched with default flags (`hs-repl` computes feedback=off, the
//! bake-off "baseline" arm) ran amnesiac: every one of 50 model calls
//! received exactly [MISSION][ATTEMPT] - no transcript, no ledger, no
//! doom-loop nudges, no convergence note. The model re-discovered the
//! empty workspace 50 times (`input_tokens` constant at 2992 across all
//! calls, `n_msgs=2`) and wrote nothing.
//!
//! Production contract pinned here: a mission run through the REPL
//! session (`run_goal`) ALWAYS carries mission memory - the transcript
//! of its own prior steps as native assistant/tool pairs, and the
//! doom-loop recovery nudge - regardless of the feedback flag, which
//! belongs to the experiment binaries (hs-swe-run/hs-tb-run
//! `--feedback off` = baseline arm), not to production missions.

use hs_loop::repl::load_session;

static SEQMODEL_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

const REPEXEC: &str = "/mnt/instinct-nvme/hairspring/target/debug/hs-plugin-repoexec";
const ANSWERSUBMIT: &str = "/mnt/instinct-nvme/hairspring/target/debug/hs-plugin-answersubmit";
const SELFCHECK: &str = "/mnt/instinct-nvme/hairspring/target/debug/hs-plugin-selfcheck";
const SCRIPTED: &str = "/mnt/instinct-nvme/hairspring/target/debug/hs-plugin-scripted";

fn rig(dir: &std::path::Path, script_lines: &[&str]) -> std::path::PathBuf {
    let _ = std::fs::remove_dir_all(dir);
    std::fs::create_dir_all(dir).unwrap();
    let toml = format!(
        r#"
[[tools]]
name = "repo.exec"
command = ["{REPEXEC}"]
subjects = ["*"]

[[tools]]
name = "answer.submit"
command = ["{ANSWERSUBMIT}"]
subjects = ["*"]

[[tools]]
name = "checker.run"
command = ["{SELFCHECK}"]
subjects = ["*"]

[[models]]
name = "scripted"
command = ["{SCRIPTED}"]
default = true
subjects = ["*"]
"#
    );
    std::fs::write(dir.join("hairspring.toml"), toml).unwrap();
    let script = dir.join("script.jsonl");
    std::fs::write(&script, script_lines.join("\n") + "\n").unwrap();
    // FIXME: Audit that the environment access only happens in single-threaded code.
    unsafe {
        std::env::set_var("HS_ANSWER_RAW", "1");
        std::env::set_var("HS_SCRIPTED_PROMPT_AWARE", "1");
        std::env::set_var("HS_SEQMODEL_SCRIPT", &script);
    }
    script
}

/// Read the mission stream's `ModelCall` prompts back, verbatim.
fn model_call_prompts(
    log_root: &std::path::Path,
    sid: uuid::Uuid,
) -> Vec<serde_json::Value> {
    let reader = hs_log::StreamReader::open(log_root, sid).unwrap();
    let events = reader.events().unwrap();
    events
        .iter()
        .filter(|e| e.kind == hs_core::EventKind::ModelCall)
        .map(|e| {
            let p = String::from_utf8_lossy(&reader.resolve_payload(e).unwrap()).to_string();
            serde_json::from_str(&p).unwrap()
        })
        .filter(|v: &serde_json::Value| v["role"].as_str() != Some("verifier"))
        .collect()
}

#[test]
fn d8_repl_mission_carries_its_own_transcript() {
    let _g = SEQMODEL_LOCK.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let dir = std::env::temp_dir().join("repl-transcript-d8");
    let _script = rig(
        &dir,
        &[
            "{\"tool\":\"repo.exec\",\"args\":{\"command\":\"echo step-one\"}}",
            "{\"tool\":\"repo.exec\",\"args\":{\"command\":\"echo step-two\"}}",
        ],
    );
    // feedback=false: exactly what `hs-repl` computes when the operator
    // passes no `--feedback on` (hs-repl.rs: `== Some("on")` on a missing
    // flag). This IS the production default.
    let mut s = load_session(
        &dir.join("hairspring.toml"),
        &dir.join("run"),
        false, Some(4),
        None,
        None,
    )
    .unwrap();
    let _ = s.run_goal("memory fixture: run two echo probes").unwrap();
    let prompts = model_call_prompts(&dir.join("run"), s.stream_id());
    assert!(prompts.len() >= 2, "two model calls must exist: {prompts:?}");
    let step2 = &prompts[1];
    let messages = step2["messages"].as_array().expect("native messages");
    let roles: Vec<&str> = messages.iter().filter_map(|m| m["role"].as_str()).collect();
    assert!(
        roles.contains(&"assistant") && roles.contains(&"tool"),
        "step 2 must carry step 1's history as native assistant/tool pairs \
         (amnesiac mission: roles were {roles:?})"
    );
}

#[test]
fn d9_repl_mission_doom_loop_nudge_reaches_the_model() {
    let _g = SEQMODEL_LOCK.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let dir = std::env::temp_dir().join("repl-doom-d9");
    // Same args, 6 times: doom window 8 / threshold 3 must fire by call 4.
    let _script = rig(
        &dir,
        &[ "{\"tool\":\"repo.exec\",\"args\":{\"command\":\"pwd\"}}"; 6 ],
    );
    let mut s = load_session(
        &dir.join("hairspring.toml"),
        &dir.join("run"),
        false, Some(8),
        None,
        None,
    )
    .unwrap();
    // repo.exec clones the workspace as a scratch git repo (the REPL
    // creates run/work but does not git-init it; the operator's real
    // missions run against a git workdir, as realrun2's did).
    let work = dir.join("run/work");
    for cmd in [
        "init -q",
        "-c user.email=t@t -c user.name=t commit -q --allow-empty -m init",
    ] {
        let st = std::process::Command::new("git")
            .args(cmd.split(' '))
            .current_dir(&work)
            .status()
            .unwrap();
        assert!(st.success(), "git {cmd} in {}", work.display());
    }
    let _ = s.run_goal("doom fixture: repeat pwd").unwrap();
    let prompts = model_call_prompts(&dir.join("run"), s.stream_id());
    assert!(prompts.len() >= 4, "four model calls must exist: {}", prompts.len());
    let rendered: Vec<String> = prompts
        .iter()
        .map(hs_loop::msgfmt::prompt_view)
        .collect();
    let dump: Vec<String> = rendered
        .iter()
        .enumerate()
        .map(|(i, p)| format!("--- frame {i} ---\n{}", &p[..p.len().min(400)]))
        .collect();
    assert!(
        rendered[3..].iter().any(|p| p.contains("DOOM LOOP")),
        "the doom-loop nudge must reach the model under the production default \
         (no nudge in any frame after the repeat threshold)\n{}",
        dump.join("\n")
    );
}
