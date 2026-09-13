//! D10 RED (dance #95, live burn 2026-09-09, realrun3): repl.rs
//! `wire_tool_env` sets `HS_SELFCHECK_DIRECT=1` UNCONDITIONALLY - the tb
//! surface's shape (term.exec on the live machine). A REPL config that
//! registers edit.patch runs the CANDIDATE surface: the model's files
//! (and its `.hs/checks`) live in the candidate worktree, so the
//! checker in direct mode reads run/work/.hs/checks, never finds them,
//! and every submission comes back "no checks declared" - a correct
//! mission cannot go green. Live symptom: run3's model wrote a working
//! 442-line pocket.py on its second call and the mission's stop
//! authority was wired to a directory the model can never write.
//! THE LAW: direct mode is bound to the LIVE-MACHINE surface (term.exec
//! registered); a candidate surface (edit.patch/edit.anchor/edit.apply
//! registered) runs the checker against the candidate.

use hs_loop::repl::load_session;

static SEQMODEL_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

const APPLYPATCH: &str = "/mnt/instinct-nvme/hairspring/target/debug/hs-plugin-applypatch";
const ANSWERSUBMIT: &str = "/mnt/instinct-nvme/hairspring/target/debug/hs-plugin-answersubmit";
const SELFCHECK: &str = "/mnt/instinct-nvme/hairspring/target/debug/hs-plugin-selfcheck";
const SCRIPTED: &str = "/mnt/instinct-nvme/hairspring/target/debug/hs-plugin-scripted";

#[test]
fn d10_candidate_surface_checker_sees_declared_checks() {
    let _g = SEQMODEL_LOCK.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let dir = std::env::temp_dir().join("repl-selfcheck-d10");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let toml = format!(
        r#"
[[tools]]
name = "edit.patch"
command = ["{APPLYPATCH}"]
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
    std::fs::write(dir.join("hairspring.toml"), &toml).unwrap();
    // One scripted step: declare a trivially-green check in the CANDIDATE.
    // Script exhausts -> the prompt-aware model submits the answer itself.
    let script = dir.join("script.jsonl");
    std::fs::write(
        &script,
        "{\"tool\":\"edit.patch\",\"args\":{\"patch\":\"*** Begin Patch\\n*** Add File: answer.py\\n+print('ok')\\n*** Add File: .hs/checks\\n+true\\n*** End Patch\"}}\n",
    )
    .unwrap();
    // FIXME: Audit that the environment access only happens in single-threaded code.
    unsafe {
        std::env::remove_var("HS_ANSWER_RAW");
        std::env::remove_var("HS_SELFCHECK_DIRECT");
        std::env::set_var("HS_SCRIPTED_PROMPT_AWARE", "1");
        std::env::set_var("HS_SEQMODEL_SCRIPT", &script);
    }
    // Candidate worktrees persist by workspace-path hash under tmp; a
    // prior run of this test leaves a stale candidate whose gitdir
    // points at the deleted base - clean it (production note: base repo
    // deleted+recreated at the SAME path leaves a dangling candidate;
    // reported as a D11 observation, out of scope here).
    let work_pre = dir.join("run/work");
    std::fs::create_dir_all(&work_pre).unwrap();
    let _ = std::fs::remove_dir_all(hs_loop::editapply::candidate_dir(&work_pre));
    let mut s = load_session(
        &dir.join("hairspring.toml"),
        &dir.join("run"),
        false, Some(12),
        None,
        None,
    )
    .unwrap();
    // The candidate surface needs a git workspace, as real missions have.
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
    let r = s.run_goal("d10 fixture: candidate checks must be seen").unwrap();
    assert!(
        r.passed,
        "a mission with a green declared check in the candidate must pass: {r:?}"
    );
}
