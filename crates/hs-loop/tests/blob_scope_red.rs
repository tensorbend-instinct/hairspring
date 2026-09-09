//! D6 RED (Eric 2026-09-09 postmortem, live `DeepSeek` burn): the REPL
//! session wired `HS_SWE_WORKSPACE`/`HS_TERM_WORKDIR` to the session's
//! log ROOT - the run dir that holds the harness's own state
//! (streams/, blobs/, memory.db, repl-hairspring.toml). repo.read and
//! repo.search are sandboxed to that anchor, so the model could read
//! its own mission text and other streams' dispatch records through the
//! repo tools - a visibility leak of harness internals into the mission.
//! THE LAW after D6: repo tools anchor at the session's WORK area
//! (`<log_root>/work`); harness state is outside the anchor by
//! construction.

use hs_loop::repl::ReplSession;

// Session env wiring (HS_SWE_WORKSPACE/HS_TERM_WORKDIR) is process-global;
// serialize the two tests that install sessions.
static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

const BENCHMODEL: &str = env!("CARGO_BIN_EXE_hs-plugin-benchmodel");
const ANSWER: &str = env!("CARGO_BIN_EXE_hs-plugin-answer");
const CHECKER: &str = env!("CARGO_BIN_EXE_hs-plugin-checker");

fn config(dir: &std::path::Path) -> std::path::PathBuf {
    let p = dir.join("hairspring.toml");
    std::fs::write(
        &p,
        format!(
            r#"
[[tools]]
name = "answer.write"
command = ["{ANSWER}"]
subjects = ["*"]

[[tools]]
name = "checker.run"
command = ["{CHECKER}"]
subjects = ["*"]

[[models]]
name = "benchmodel"
command = ["{BENCHMODEL}"]
default = true
"#
        ),
    )
    .unwrap();
    p
}

#[test]
fn session_anchors_repo_tools_at_work_area_not_run_root() {
    let _g = ENV_LOCK.lock().unwrap();
    let dir = tempfile::tempdir().unwrap();
    let log = tempfile::tempdir().unwrap();
    let _session = ReplSession::load(&config(dir.path()), log.path(), false, 4).unwrap();
    let ws = std::env::var("HS_SWE_WORKSPACE").unwrap();
    let expected = log.path().join("work");
    assert_eq!(
        std::path::Path::new(&ws),
        expected.as_path(),
        "repo tools must anchor at the work area, not the run root"
    );
    let tw = std::env::var("HS_TERM_WORKDIR").unwrap();
    assert_eq!(std::path::Path::new(&tw), expected.as_path());
}

#[test]
fn repo_read_cannot_reach_harness_state_under_run_root() {
    let _g = ENV_LOCK.lock().unwrap();
    let dir = tempfile::tempdir().unwrap();
    let log = tempfile::tempdir().unwrap();
    let _session = ReplSession::load(&config(dir.path()), log.path(), false, 4).unwrap();
    // harness state under the run root, the way a live run lays it out
    let streams = log.path().join("streams");
    std::fs::create_dir_all(&streams).unwrap();
    std::fs::write(streams.join("dispatch.txt"), "MISSION-TEXT-SECRET").unwrap();
    let ws = std::path::PathBuf::from(std::env::var("HS_SWE_WORKSPACE").unwrap());
    let r = hs_loop::repotools::read_repo_window(&ws, "../streams/dispatch.txt", None, None);
    assert!(r.is_err(), "harness state must be outside the anchor: {r:?}");
}
