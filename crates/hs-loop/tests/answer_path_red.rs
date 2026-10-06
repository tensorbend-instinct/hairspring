//! The answer file follows the ACTIVE project folder (dsh workspace parity).
//! Bug: a mission in workspace /proj wrote answer.txt under the engine's old
//! log_root/work, so the checker (cwd = /proj) never saw it and the critic
//! refuted forever. The root is session state, never read from process env
//! at mission time (parallel sessions in one process must not leak).
#[test]
fn answer_path_lives_under_the_session_project_root() {
    let log = tempfile::tempdir().unwrap();
    let proj = tempfile::tempdir().unwrap();
    let p = hs_loop::mission_answer_path(log.path(), Some(proj.path()), "m1");
    assert_eq!(p, proj.path().join("m1").join("answer.txt"));
    let p = hs_loop::mission_answer_path(log.path(), None, "m1");
    assert_eq!(p, log.path().join("work").join("m1").join("answer.txt"), "no project root: legacy location");
}

#[test]
fn engine_mission_writes_its_answer_dir_inside_the_project_folder() {
    use hs_loop::engine::Engine;
    const ANSWER: &str = env!("CARGO_BIN_EXE_hs-plugin-answer");
    const CHECKER: &str = env!("CARGO_BIN_EXE_hs-plugin-liechecker");
    const SCRIPTED: &str = env!("CARGO_BIN_EXE_hs-plugin-scripted");
    let (d, log, proj) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    let cfg = d.path().join("hairspring.toml");
    std::fs::write(&cfg, format!("[[tools]]\nname = \"answer.write\"\ncommand = [\"{ANSWER}\"]\nsubjects = [\"*\"]\n\n[[tools]]\nname = \"checker.run\"\ncommand = [\"{CHECKER}\"]\nsubjects = [\"*\"]\n\n[[models]]\nname = \"scripted\"\ncommand = [\"{SCRIPTED}\"]\ndefault = true\n")).unwrap();
    let script = d.path().join("s.jsonl");
    std::fs::write(&script, format!("{{\"tool\":\"answer.write\",\"args\":{{\"path\":\"{}\",\"content\":\"X\"}}}}\n", d.path().join("a.txt").display())).unwrap();
    unsafe {
        std::env::set_var("HS_SEQMODEL_SCRIPT", &script);
        std::env::set_var("HS_PROJECT_ROOT", proj.path());
    }
    let mut eng = Engine::open(&cfg, log.path(), Some(3)).unwrap();
    eng.run_goal("slugged goal", |_| {}).unwrap();
    unsafe { std::env::remove_var("HS_PROJECT_ROOT") };
    let in_proj = proj.path().join(hs_loop::repl::goal_slug("slugged goal"));
    assert!(in_proj.is_dir(), "answer dir created inside the project folder: {:?}", std::fs::read_dir(proj.path()).map(|r| r.flatten().map(|e| e.file_name()).collect::<Vec<_>>()));
    assert!(!log.path().join("work").join(hs_loop::repl::goal_slug("slugged goal")).exists());
}
