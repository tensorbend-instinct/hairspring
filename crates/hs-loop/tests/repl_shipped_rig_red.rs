//! The SHIPPED first-run rig (hairspring.example.toml, tb surface):
//! term.exec on the live machine + blind self-checker. RED before the
//! fix: the REPL never set `HS_SELFCHECK_DIRECT`, so checker.run looked
//! for an edit.patch candidate the tb surface can never materialize
//! and EVERY submission came back "no checks declared" - a user
//! following the shipped config could never pass a mission (the exact
//! "can't get it set up" brokenness, pinned).

use hs_loop::repl::load_session;

#[test]
fn r1_shipped_tb_rig_passes_a_mission_end_to_end() {
    let dir = std::env::temp_dir().join("repl-shipped-rig-r1");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    // Mirror of hairspring.example.toml with debug-binary paths (the
    // shipped file's @PREFIX@ becomes the install prefix at install).
    let toml = r#"
[[tools]]
name = "term.exec"
command = ["/mnt/instinct-nvme/hairspring/target/debug/hs-plugin-termexec"]
subjects = ["*"]

[[tools]]
name = "notes.scratch"
command = ["/mnt/instinct-nvme/hairspring/target/debug/hs-plugin-notescratch"]
subjects = ["*"]

[[tools]]
name = "answer.submit"
command = ["/mnt/instinct-nvme/hairspring/target/debug/hs-plugin-answersubmit"]
subjects = ["*"]

[[tools]]
name = "checker.run"
command = ["/mnt/instinct-nvme/hairspring/target/debug/hs-plugin-selfcheck"]
subjects = ["*"]

[[models]]
name = "scripted"
command = ["/mnt/instinct-nvme/hairspring/target/debug/hs-plugin-scripted"]
default = true
subjects = ["*"]
"#;
    std::fs::write(dir.join("hairspring.toml"), toml).unwrap();
    // The agent declares its own check on the live machine (direct
    // mode: .hs/checks in the workdir), then submits.
    std::fs::write(
        dir.join("script.jsonl"),
        "{\"tool\":\"term.exec\",\"args\":{\"command\":\"mkdir -p .hs && printf 'true\\\\n' > .hs/checks && cat .hs/checks\"}}\n",
    )
    .unwrap();
    std::env::remove_var("HS_SELFCHECK_DIRECT");
    std::env::set_var("HS_ANSWER_RAW", "1");
    std::env::set_var("HS_SCRIPTED_PROMPT_AWARE", "1");
    std::env::set_var("HS_SEQMODEL_SCRIPT", dir.join("script.jsonl"));

    let mut s = load_session(&dir.join("hairspring.toml"), &dir.join("run"), false, 12, None, None)
        .unwrap();
    let r = s.run_goal("shipped rig proof").unwrap();
    assert!(
        r.passed,
        "the shipped tb rig passes: declare checks with term.exec, submit, checker green: {r:?}"
    );
    assert!(
        dir.join("run/.hs/checks").exists(),
        "checks declared on the live workdir"
    );
}
