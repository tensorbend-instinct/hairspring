//! Eric's five #2/#3 (2026-09-08): scripted-fixture missions end
//! `verifier_malfunction` / `steps_exhausted` - fake failures in every
//! proof shot. Diagnosis from the code path: after the checker goes
//! green the loop calls the model with the verdict.submit tool; the
//! scripted provider plays the next SCRIPT LINE regardless of what
//! the request asks, so the verifier gets prose and the mission books
//! `verifier_malfunction`. And when the script runs out, the provider
//! repeats its last prose line forever - no answer is ever submitted,
//! so the mission burns to `steps_exhausted`. A fixture that cannot
//! answer "audit this" or "submit your answer" is a toy model.
//!
//! Second defect found by the first RED: the fixture config offered
//! answer.write while the loop's native schema (`tb_tools`) offers
//! answer.submit - the model-facing contract is answer.submit
//! {path, summary}; the fixture now wires THAT plugin.
//!
//! Contract: the scripted provider is PROMPT-AWARE. Offered the
//! verdict.submit tool it returns a well-formed not-refuted verdict
//! (a competent model auditing honest work); when the script runs out
//! on an operator prompt carrying `ANSWER_PATH`, it submits the answer
//! with the offered answer tool, then stands down once the ARTIFACT
//! block shows the answer on disk.

static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn load(dir: &std::path::Path) -> hs_loop::repl::ReplSession {
    std::fs::create_dir_all(dir).unwrap();
    std::fs::write(
        dir.join("hairspring.toml"),
        r#"
[[tools]]
name = "answer.submit"
command = ["/mnt/instinct-nvme/hairspring/target/debug/hs-plugin-answersubmit"]
subjects = ["*"]
[[tools]]
name = "checker.run"
command = ["/mnt/instinct-nvme/hairspring/target/debug/hs-plugin-liechecker"]
subjects = ["*"]
[[models]]
name = "scripted"
command = ["/mnt/instinct-nvme/hairspring/target/debug/hs-plugin-scripted"]
default = true
"#,
    )
    .unwrap();
    hs_loop::repl::load_session(&dir.join("hairspring.toml"), &dir.join("run"), false, 5, None, None)
        .unwrap()
}

// R1: an honest mission PASSES - checker green, verifier not-refuted,
// outcome "verified", answer artifact on disk.
#[test]
fn r1_mission_passes_honestly() {
    let _guard = ENV_LOCK.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    // answersubmit's tb mode (the mode the TUI runs in): the submission is a
    // completion summary, not a git diff (that path needs HS_SWE_WORKSPACE).
    std::env::set_var("HS_ANSWER_RAW", "1");
    std::env::set_var("HS_SCRIPTED_PROMPT_AWARE", "1");
    let dir = std::env::temp_dir().join("fixture-honesty-r1");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    // One prose line, then the script is DONE - everything after is
    // the provider's own competence.
    std::fs::write(dir.join("script.jsonl"), "reading the code\n").unwrap();
    unsafe {
        std::env::set_var("HS_SEQMODEL_SCRIPT", dir.join("script.jsonl"));
    }
    let mut s = load(&dir);
    let r = s.run_goal("fix the lexer").unwrap();
    assert!(r.passed, "an honest scripted mission passes: {r:?}");
    assert_eq!(r.outcome, "verified", "not a fake malfunction: {r:?}");
    let artifact = std::fs::read_to_string(&r.answer_path).unwrap_or_default();
    assert!(
        !artifact.is_empty(),
        "the answer artifact exists at the MISSION's answer_path ({:?}) - :last's happy path needs it",
        r.answer_path
    );
}

// R2: a second mission with the script long exhausted also passes -
// no steps_exhausted fake failure.
#[test]
fn r2_exhausted_script_still_lands() {
    let _guard = ENV_LOCK.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    std::env::set_var("HS_ANSWER_RAW", "1");
    std::env::set_var("HS_SCRIPTED_PROMPT_AWARE", "1");
    let dir = std::env::temp_dir().join("fixture-honesty-r2");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("script.jsonl"), "reading the code\n").unwrap();
    unsafe {
        std::env::set_var("HS_SEQMODEL_SCRIPT", dir.join("script.jsonl"));
    }
    let mut s = load(&dir);
    let r1 = s.run_goal("first goal").unwrap();
    let r2 = s.run_goal("second goal").unwrap();
    assert!(r1.passed && r2.passed, "both pass: {r1:?} {r2:?}");
    assert_eq!(r1.outcome, "verified");
    assert_eq!(r2.outcome, "verified", "script exhaustion is not failure");
}
