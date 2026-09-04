//! Faithful-loop contract (Eric's 2026-09-04 order): the mission loop must
//! (a) run the checker ONLY after answer.write - not after read/search
//! tools, whose results are the point of the step; (b) feed successful tool
//! results back into the next step's context, or read/search tools are
//! decoration and the model loops blind (observed: 19 identical repo.read
//! calls in one mission, 2026-09-04).
use hs_loop::*;

const ANSWER: &str = env!("CARGO_BIN_EXE_hs-plugin-answer");
const CHECKER: &str = env!("CARGO_BIN_EXE_hs-plugin-checker");
const SEQMODEL: &str = env!("CARGO_BIN_EXE_hs-plugin-seqmodel");
const PROBE: &str = env!("CARGO_BIN_EXE_hs-plugin-probe");

fn rig(dir: &std::path::Path, log: &std::path::Path) -> InnerLoop {
    let config = dir.join("hairspring.toml");
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
command = ["{CHECKER}"]
subjects = ["*"]

[[tools]]
name = "probe.read"
command = ["{PROBE}"]
subjects = ["*"]

[[models]]
name = "seqmodel"
command = ["{SEQMODEL}"]
default = true
"#
        ),
    )
    .unwrap();
    let kernel = hs_kernel::Kernel::load(&config).unwrap();
    InnerLoop::new(kernel, log, true, 6).unwrap()
}

#[test]
fn tool_results_reach_the_next_step_and_checker_runs_only_after_write() {
    let dir = tempfile::tempdir().unwrap();
    let log = tempfile::tempdir().unwrap();
    let mut l = rig(dir.path(), log.path());
    let r = l.run_mission("task-0").unwrap();
    assert!(
        r.passed,
        "seqmodel writes the token only if step 1's probe result reached step 2"
    );
    assert_eq!(r.steps, 2, "probe, then write, then auto-checker");

    let sid = {
        let mut v: Vec<_> = std::fs::read_dir(log.path().join("streams"))
            .unwrap()
            .map(|e| e.unwrap().file_name().into_string().unwrap())
            .collect();
        assert_eq!(v.len(), 1);
        uuid::Uuid::parse_str(&v.pop().unwrap()).unwrap()
    };
    let evs = hs_log::StreamReader::open(log.path(), sid)
        .unwrap()
        .events()
        .unwrap();
    let checker_runs = evs
        .iter()
        .filter(|e| e.kind == hs_core::EventKind::Feedback)
        .count();
    assert_eq!(
        checker_runs, 1,
        "checker must run exactly once: after answer.write, never after probe.read"
    );
    let probe_calls = evs
        .iter()
        .filter(|e| e.kind == hs_core::EventKind::ToolCall)
        .count();
    assert_eq!(probe_calls, 2, "probe.read + answer.write");
}
