//! The answer file follows the ACTIVE project folder (dsh workspace parity).
//! Bug: a mission in workspace /proj wrote answer.txt under the engine's old
//! log_root/work, so the checker (cwd = /proj) never saw it and the critic
//! refuted forever.
#[test]
fn answer_path_lives_under_the_effective_project_root() {
    let log = tempfile::tempdir().unwrap();
    let proj = tempfile::tempdir().unwrap();
    unsafe {
        std::env::set_var("HS_PROJECT_ROOT_EFFECTIVE", proj.path());
        std::env::remove_var("HS_PROJECT_ROOT");
    }
    let p = hs_loop::mission_answer_path(log.path(), "m1");
    assert_eq!(p, proj.path().canonicalize().unwrap().join("m1").join("answer.txt"));
    unsafe { std::env::remove_var("HS_PROJECT_ROOT_EFFECTIVE") };
    let p = hs_loop::mission_answer_path(log.path(), "m1");
    assert_eq!(p, log.path().join("work").join("m1").join("answer.txt"), "no project root: legacy location");
}
